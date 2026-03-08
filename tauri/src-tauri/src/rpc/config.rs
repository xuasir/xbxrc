use crate::AppState;
use serde_json::Value;
use tauri::Manager;

pub async fn handle_config_rpc(
    method: &str,
    params: Option<Value>,
    app_handle: tauri::AppHandle,
) -> Result<Value, String> {
    let state = app_handle.state::<AppState>();
    let service = state.config.read().await;

    match method {
        "get" => {
            let keys = params
                .as_ref()
                .and_then(|payload| payload.get("keys"))
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|item| item.as_str().map(ToString::to_string))
                .collect::<Vec<_>>();

            service.get_by_keys(&keys)
        }
        "set" => {
            let patch = params
                .as_ref()
                .and_then(|payload| payload.get("patch"))
                .and_then(|value| value.as_object())
                .cloned()
                .unwrap_or_default();

            service.set_by_patch(&patch)
        }
        "getGroups" => service.get_groups(),
        _ => Err(format!("Unknown method in config: {}", method)),
    }
}
