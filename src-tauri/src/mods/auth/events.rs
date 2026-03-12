use crate::mods::auth::types::AuthState;
use serde_json::json;
use tauri::AppHandle;

pub const AUTH_SESSION_READY_CHANNEL: &str = "xbxrc:auth:session-ready";
pub const AUTH_STATE_CHANGED_CHANNEL: &str = "xbxrc:auth:state-changed";

fn normalize_provider(provider: &str) -> &str {
    if provider == "xal" {
        return "xal";
    }
    "xal"
}

/// 发出 auth 会话就绪事件，供 renderer 触发用户态同步。
pub fn emit_session_ready(
    app_handle: &AppHandle,
    provider: &str,
    app_level: u32,
) -> Result<(), String> {
    crate::event_bridge::emit(
        app_handle,
        AUTH_SESSION_READY_CHANNEL,
        &json!({
            "provider": normalize_provider(provider),
            "appLevel": app_level,
            "at": chrono::Utc::now().to_rfc3339()
        }),
    )
}

/// 发出 auth 状态变化事件。
pub fn emit_auth_state_changed(app_handle: &AppHandle, state: &AuthState) -> Result<(), String> {
    crate::event_bridge::emit(app_handle, AUTH_STATE_CHANGED_CHANNEL, state)
}
