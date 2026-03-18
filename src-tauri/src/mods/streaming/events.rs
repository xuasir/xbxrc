use crate::mods::streaming::types::StreamingStartupEvent;
use tauri::AppHandle;

pub const STREAMING_STARTUP_EVENT_CHANNEL: &str = "xbxrc:streaming:startup-event";

pub fn emit_startup_event(
    app_handle: &AppHandle,
    payload: &StreamingStartupEvent,
) -> Result<(), String> {
    crate::event_bridge::emit(app_handle, STREAMING_STARTUP_EVENT_CHANNEL, payload)
}
