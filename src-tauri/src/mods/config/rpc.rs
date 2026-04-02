use crate::error::AppResult;
use crate::AppState;
use serde::Deserialize;
use serde_json::Value;
use tauri::Manager;

fn apply_runtime_trace_mode_live(app_handle: &tauri::AppHandle, mode: &str) -> AppResult<()> {
    let state = app_handle.state::<AppState>();
    state
        .runtime_trace
        .apply_trace_mode(mode)
        .map_err(|error| {
            crate::error::AppError::Internal(format!("runtime trace mode apply failed: {error}"))
        })?;
    crate::mods::runtime_trace::apply_xbxengine_trace_logging(mode);
    let interval = crate::mods::runtime_trace::stats_snapshot_interval(mode);
    state.xbxengine.set_stats_snapshot_interval(interval);
    log::info!("Runtime trace mode applied live (mode={})", mode);
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "camelCase")]
pub enum ConfigCommand {
    Get {
        keys: Vec<String>,
    },
    Set {
        patch: serde_json::Map<String, Value>,
    },
    GetGroups,
}

pub async fn handle_rpc(
    method: &str,
    params: Option<Value>,
    app_handle: tauri::AppHandle,
) -> AppResult<Value> {
    let state = app_handle.state::<AppState>();
    let service = state.config.clone();

    // 转换到强类型命令
    let json_cmd = match params {
        Some(p) => serde_json::json!({ "method": method, "params": p }),
        None => serde_json::json!({ "method": method }),
    };

    let command: ConfigCommand = serde_json::from_value(json_cmd).map_err(|e| {
        crate::error::AppError::InvalidParams(format!("Invalid config command params: {}", e))
    })?;

    match command {
        ConfigCommand::Get { keys } => service.get_by_keys(&keys).map_err(Into::into),
        ConfigCommand::Set { patch } => {
            let touch_runtime_trace = patch.contains_key("runtime_trace_mode");
            let result = service
                .set_by_patch(&patch)
                .map_err(crate::error::AppError::from)?;
            if touch_runtime_trace {
                if let Some(mode) = result
                    .get("runtime_trace_mode")
                    .and_then(|value| value.as_str())
                {
                    apply_runtime_trace_mode_live(&app_handle, mode)?;
                }
            }
            Ok(result)
        }
        ConfigCommand::GetGroups => service.get_groups().map_err(Into::into),
    }
}
