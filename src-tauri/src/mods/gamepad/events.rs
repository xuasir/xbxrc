use serde_json::Value;
use tauri::AppHandle;

pub const GAMEPAD_RUNTIME_SNAPSHOT_CHANNEL: &str = "xbxrc:gamepad:runtime-snapshot";

pub fn emit_runtime_snapshot(app_handle: &AppHandle, snapshot: &Value) -> Result<(), String> {
    crate::event_bridge::emit_json(app_handle, GAMEPAD_RUNTIME_SNAPSHOT_CHANNEL, snapshot)
}
