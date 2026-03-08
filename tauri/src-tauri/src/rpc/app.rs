use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

pub async fn handle_app_rpc(
    method: &str,
    params: Option<Value>,
    app_handle: AppHandle,
) -> Result<Value, String> {
    let state = app_handle.state::<crate::AppState>();
    let service = state.app_state.clone();

    match method {
        "getVersion" => Ok(json!(service.get_version())),
        "ping" => {
            let message = params
                .as_ref()
                .and_then(|payload| payload.get("message"))
                .and_then(|value| value.as_str())
                .ok_or("Missing message parameter")?;

            serde_json::to_value(service.ping(message)).map_err(|error| error.to_string())
        }
        "isFullscreen" => Ok(json!(service.is_fullscreen())),
        "toggleFullscreen" => Ok(json!(service.toggle_fullscreen()?)),
        "enterFullscreen" => Ok(json!(service.enter_fullscreen()?)),
        "exitFullscreen" => Ok(json!(service.exit_fullscreen()?)),
        "getStartupFlags" => serde_json::to_value(service.get_startup_flags().await)
            .map_err(|error| error.to_string()),
        "resetAutoConnect" => Ok(json!({ "reset": service.reset_auto_connect().await })),
        "clearUserData" => {
            let result = service.clear_user_data().await?;
            serde_json::to_value(result).map_err(|error| error.to_string())
        }
        "clearData" => {
            let result = service.clear_data().await?;
            service.restart_delayed(10).await;
            Ok(json!({
                "cleared": result.cleared,
                "legacyStateCleared": result.legacy_state_cleared,
                "restarted": true
            }))
        }
        "restart" => {
            service.restart().await;
            #[allow(unreachable_code)]
            Ok(json!({ "accepted": true }))
        }
        "quit" => {
            service.quit().await;
            #[allow(unreachable_code)]
            Ok(json!({ "accepted": true }))
        }
        _ => Err(format!("Unknown method in app: {}", method)),
    }
}
