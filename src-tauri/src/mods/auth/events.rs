use serde_json::json;
use tauri::AppHandle;

pub const AUTH_SESSION_READY_CHANNEL: &str = "auth.sessionReady";

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
