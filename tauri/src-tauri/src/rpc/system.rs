use crate::AppState;
use tauri::{AppHandle, Manager};

pub async fn handle_system_rpc(
    method: &str,
    params: Option<serde_json::Value>,
    app_handle: AppHandle,
) -> Result<serde_json::Value, String> {
    let state = app_handle.state::<AppState>();
    let service = state.app_state.clone();

    match method {
        "openExternal" => {
            let url = params
                .as_ref()
                .and_then(|payload| payload.get("url"))
                .and_then(|value| value.as_str())
                .ok_or("Missing url parameter")?;
            service.open_external(url)?;
            Ok(serde_json::json!({}))
        }
        _ => Err(format!("Unknown method in system: {}", method)),
    }
}
