use crate::error::AppResult;
use crate::AppState;
use serde::Deserialize;
use serde_json::Value;
use tauri::Manager;

fn apply_release_runtime_trace_patch_clamp(
    patch: serde_json::Map<String, Value>,
) -> serde_json::Map<String, Value> {
    #[cfg(not(debug_assertions))]
    {
        let mut patch = patch;
        if let Some(mode) = patch.get("runtime_trace_mode").and_then(Value::as_str) {
            let effective_mode = crate::mods::runtime_trace::effective_runtime_trace_mode(mode);
            patch.insert(
                "runtime_trace_mode".to_string(),
                Value::String(effective_mode),
            );
        }
        patch.remove("runtime_trace_dimensions");
        patch
    }
    #[cfg(debug_assertions)]
    {
        patch
    }
}

fn apply_runtime_trace_config_live(app_handle: &tauri::AppHandle, config: &Value) -> AppResult<()> {
    let stored_mode = config
        .get("runtime_trace_mode")
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            if cfg!(debug_assertions) {
                "dev"
            } else {
                "production"
            }
        });
    let mode = crate::mods::runtime_trace::effective_runtime_trace_mode(stored_mode);
    let dimensions = if cfg!(debug_assertions) {
        std::env::var("XBX_TRACE_DIMENSIONS")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                config
                    .get("runtime_trace_dimensions")
                    .and_then(Value::as_str)
                    .map(std::string::ToString::to_string)
            })
    } else {
        None
    };
    let state = app_handle.state::<AppState>();
    state
        .runtime_trace
        .apply_trace_config(&mode, dimensions.as_deref())
        .map_err(|error| {
            crate::error::AppError::Internal(format!("runtime trace mode apply failed: {error}"))
        })?;
    crate::mods::runtime_trace::apply_xbxengine_trace_logging(
        state.runtime_trace.trace_profile(),
        state.runtime_trace.trace_dimensions(),
    );
    let interval =
        crate::mods::runtime_trace::stats_snapshot_interval(state.runtime_trace.trace_profile());
    state.xbxengine.set_stats_snapshot_interval(interval);
    log::debug!(
        "Runtime trace mode applied live (mode={}, dimensions={})",
        mode,
        state.runtime_trace.trace_dimensions().expression()
    );
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
            let patch = apply_release_runtime_trace_patch_clamp(patch);
            let touch_runtime_trace = patch.contains_key("runtime_trace_mode")
                || patch.contains_key("runtime_trace_dimensions");
            let result = service
                .set_by_patch(&patch)
                .map_err(crate::error::AppError::from)?;
            if touch_runtime_trace {
                apply_runtime_trace_config_live(&app_handle, &result)?;
            }
            Ok(result)
        }
        ConfigCommand::GetGroups => service.get_groups().map_err(Into::into),
    }
}
