//! 主窗口 gate hints 同步到 ohmygamepad（与 runtime lifecycle 一起在 host enrich 中派生 `input_gate`）。

use std::sync::atomic::{AtomicBool, Ordering};

use ohmygamepad_sdl3::ShellWindowGateHints;
use tauri::{AppHandle, Manager};

use crate::shell::state::AppState;

/// 主窗口是否处于 OS 认为的「获得焦点」态，**仅**由 `WindowEvent::Focused` 更新（诊断用）。
static SHELL_MAIN_WINDOW_FOCUSED_FROM_EVENT: AtomicBool = AtomicBool::new(false);

/// 仅应由 `lib.rs` 里 `WindowEvent::Focused` 分支调用。
pub fn record_shell_main_window_focused_from_os_event(focused: bool) {
    SHELL_MAIN_WINDOW_FOCUSED_FROM_EVENT.store(focused, Ordering::Relaxed);
}

/// 壳层主动隐藏主窗口时，需要同步清掉上一次 OS focus 事件留下的标记。
pub fn clear_shell_main_window_focus_from_shell_action() {
    SHELL_MAIN_WINDOW_FOCUSED_FROM_EVENT.store(false, Ordering::Relaxed);
}

pub fn shell_main_window_focused_from_event() -> bool {
    SHELL_MAIN_WINDOW_FOCUSED_FROM_EVENT.load(Ordering::Relaxed)
}

#[cfg(not(target_os = "windows"))]
fn build_non_windows_gate_hints(focused_from_event: bool) -> ShellWindowGateHints {
    ShellWindowGateHints {
        shell_app_active: focused_from_event,
    }
}

#[cfg(not(target_os = "windows"))]
fn build_gate_hints(
    window: &tauri::WebviewWindow,
    focused_from_event: bool,
) -> ShellWindowGateHints {
    let _ = window;
    build_non_windows_gate_hints(focused_from_event)
}

#[cfg(target_os = "windows")]
fn build_gate_hints(
    window: &tauri::WebviewWindow,
    focused_from_event: bool,
) -> ShellWindowGateHints {
    super::fse_windows::build_gate_hints(window, focused_from_event)
}

pub fn current_shell_window_gate_hints(window: &tauri::WebviewWindow) -> ShellWindowGateHints {
    build_gate_hints(window, shell_main_window_focused_from_event())
}

pub fn sync_gamepad_input_gate(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let Some(window) = app.get_webview_window("main") else {
        state
            .gamepad
            .set_shell_window_gate_hints(ShellWindowGateHints {
                shell_app_active: false,
            });
        return;
    };

    state
        .gamepad
        .set_shell_window_gate_hints(current_shell_window_gate_hints(&window));
}

#[cfg(all(test, not(target_os = "windows")))]
mod tests {
    use super::{
        build_non_windows_gate_hints, clear_shell_main_window_focus_from_shell_action,
        record_shell_main_window_focused_from_os_event, shell_main_window_focused_from_event,
    };

    #[test]
    fn non_windows_gate_ignores_visibility_and_minimize_when_focused() {
        let hints = build_non_windows_gate_hints(true);
        assert!(hints.shell_app_active);
    }

    #[test]
    fn shell_action_can_clear_stale_focus_flag() {
        record_shell_main_window_focused_from_os_event(true);
        assert!(shell_main_window_focused_from_event());

        clear_shell_main_window_focus_from_shell_action();

        assert!(!shell_main_window_focused_from_event());
    }
}
