use tauri::{AppHandle, Manager};

use crate::mods::gamepad::input_route;

use super::AppState;

fn record_gamepad_shell_trace(window: &tauri::Window, event: &str, payload: serde_json::Value) {
    let Some(app_state) = window.try_state::<AppState>() else {
        return;
    };
    app_state
        .runtime_trace
        .record_event("gamepad-shell", event, None, payload);
}

/// 采样常驻运行，壳层只维护输入路由并记录事件轨迹。
pub fn handle_gamepad_page_loaded(window: &tauri::Window, url: String) {
    record_gamepad_shell_trace(
        window,
        "pageLoad",
        serde_json::json!({
            "windowLabel": window.label(),
            "url": url,
        }),
    );
    input_route::sync_gamepad_input_route(&window.app_handle());
}

pub fn handle_gamepad_window_focus_changed(window: &tauri::Window, focused: bool) {
    record_gamepad_shell_trace(
        window,
        "windowFocused",
        serde_json::json!({
            "focused": focused,
            "windowLabel": window.label(),
        }),
    );
    // 失焦不改变 route：窗口仍在台前时手柄输入必须继续可用。
    // 只有隐藏 / 最小化才算「台后」，见 `handle_gamepad_window_resized`。
    if focused {
        input_route::sync_gamepad_input_route(&window.app_handle());
    }
}

pub fn handle_gamepad_window_resized(window: &tauri::Window) {
    match window.is_minimized() {
        Ok(minimized) => {
            record_gamepad_shell_trace(
                window,
                if minimized {
                    "windowResizedMinimized"
                } else {
                    "windowResizedRestored"
                },
                serde_json::json!({
                    "windowLabel": window.label(),
                    "minimized": minimized,
                }),
            );
            input_route::sync_gamepad_input_route(&window.app_handle());
        }
        Err(error) => {
            log::warn!("Failed to inspect window minimized state: {}", error);
        }
    }
}

pub fn handle_gamepad_app_resumed(app_handle: &AppHandle) {
    if let Some(app_state) = app_handle.try_state::<AppState>() {
        app_state.runtime_trace.record_event(
            "gamepad-shell",
            "appResumed",
            None,
            serde_json::json!({
                "source": "runEvent",
            }),
        );
    }
    input_route::sync_gamepad_input_route(app_handle);
}
