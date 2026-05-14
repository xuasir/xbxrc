//! 主窗口 hints 同步到 ohmygamepad（与 `stream_pad_forwarding` 一起在 host enrich 中派生 `input_gate`）。

use crate::shell::state::AppState;
use tauri::{AppHandle, Manager};

pub fn sync_gamepad_input_gate(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    if let Some(window) = app.get_webview_window("main") {
        let focused = window.is_focused().unwrap_or(false);
        let visible = window.is_visible().unwrap_or(false);
        let minimized = window.is_minimized().unwrap_or(false);
        state
            .gamepad
            .set_shell_window_gate_hints(focused, visible, minimized);
    } else {
        state
            .gamepad
            .set_shell_window_gate_hints(false, false, false);
    }
}
