use serde_json::Value;
use tauri::AppHandle;

// 与 renderer `src/shared/events/gamepad.ts` 保持一份稳定 channel 命名。
// 之前这里残留旧名字，导致 renderer 实际监听不到 gamepad 运行时推送。
pub const GAMEPAD_RUNTIME_SNAPSHOT_CHANNEL: &str = "xbxrc:gamepad:runtime-snapshot";
pub const GAMEPAD_DEVICES_CHANGED_CHANNEL: &str = "xbxrc:gamepad:devices-changed";
pub const GAMEPAD_PAD_SNAPSHOT_CHANNEL: &str = "xbxrc:gamepad:pad-snapshot";
pub const GAMEPAD_ROUTE_CHANGED_CHANNEL: &str = "xbxrc:gamepad:route-changed";

pub fn emit_runtime_snapshot(app_handle: &AppHandle, snapshot: &Value) -> Result<(), String> {
    crate::event_bridge::emit_json(app_handle, GAMEPAD_RUNTIME_SNAPSHOT_CHANNEL, snapshot)
}

pub fn emit_devices_changed(app_handle: &AppHandle, devices: &Value) -> Result<(), String> {
    crate::event_bridge::emit_json(app_handle, GAMEPAD_DEVICES_CHANGED_CHANNEL, devices)
}

pub fn emit_pad_snapshot(app_handle: &AppHandle, pad_snapshot: &Value) -> Result<(), String> {
    crate::event_bridge::emit_json(app_handle, GAMEPAD_PAD_SNAPSHOT_CHANNEL, pad_snapshot)
}

pub fn emit_route_changed(app_handle: &AppHandle, route_target: &Value) -> Result<(), String> {
    crate::event_bridge::emit_json(app_handle, GAMEPAD_ROUTE_CHANGED_CHANNEL, route_target)
}
