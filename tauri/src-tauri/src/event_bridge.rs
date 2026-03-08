use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};
/// event bridge 仅保留基础设施：统一发射入口。
/// 具体事件常量与映射下沉到各个 mods/events.rs。
pub fn emit_json(app_handle: &AppHandle, channel: &str, payload: &Value) -> Result<(), String> {
    app_handle
        .emit(channel, payload)
        .map_err(|error| error.to_string())
}

/// 支持直接发送可序列化对象，避免调用方手工转 Value。
pub fn emit<T: Serialize>(
    app_handle: &AppHandle,
    channel: &str,
    payload: &T,
) -> Result<(), String> {
    app_handle
        .emit(channel, payload)
        .map_err(|error| error.to_string())
}
