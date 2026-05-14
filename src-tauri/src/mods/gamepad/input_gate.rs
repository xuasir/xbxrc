//! 主窗口 hints 同步到 ohmygamepad（与 `stream_pad_forwarding` 一起在 host enrich 中派生 `input_gate`）。

use std::sync::atomic::{AtomicBool, Ordering};

use crate::shell::state::AppState;
use tauri::{AppHandle, Manager};

/// 主窗口是否处于 OS 认为的「获得焦点」态，**仅**由 `WindowEvent::Focused` 更新。
/// 不再读取 `WebviewWindow::is_focused()`（Windows WebView2 上易与真实可输入态不一致）。
static SHELL_MAIN_WINDOW_FOCUSED_FROM_EVENT: AtomicBool = AtomicBool::new(false);

/// 仅应由 `lib.rs` 里 `WindowEvent::Focused` 分支调用。
pub fn record_shell_main_window_focused_from_os_event(focused: bool) {
    SHELL_MAIN_WINDOW_FOCUSED_FROM_EVENT.store(focused, Ordering::Relaxed);
}

pub fn sync_gamepad_input_gate(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    if let Some(window) = app.get_webview_window("main") {
        let focused = SHELL_MAIN_WINDOW_FOCUSED_FROM_EVENT.load(Ordering::Relaxed);
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
