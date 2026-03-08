use crate::error::AppResult;
use crate::AppState;
use serde::Deserialize;
use serde_json::Value;
use tauri::Manager;

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
        ConfigCommand::Set { patch } => service.set_by_patch(&patch).map_err(Into::into),
        ConfigCommand::GetGroups => service.get_groups().map_err(Into::into),
    }
}
