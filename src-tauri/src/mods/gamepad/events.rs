use serde_json::Value;
use tauri::AppHandle;

pub const GAMEPAD_RUNTIME_SNAPSHOT_CHANNEL: &str = "gamepad.runtimeSnapshot";
pub const GAMEPAD_DEVICES_CHANGED_CHANNEL: &str = "gamepad.devicesChanged";
pub const GAMEPAD_PAD_SNAPSHOT_CHANNEL: &str = "gamepad.padSnapshot";
pub const GAMEPAD_ROUTE_CHANGED_CHANNEL: &str = "gamepad.routeChanged";

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
