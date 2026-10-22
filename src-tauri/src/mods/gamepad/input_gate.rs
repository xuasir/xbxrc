//! 主窗口 gate hints 同步到 ohmygamepad（与 runtime lifecycle 一起在 host enrich 中派生 `input_gate`）。

use std::sync::atomic::{AtomicBool, Ordering};

use ohmygamepad_sdl3::ShellWindowGateHints;
use tauri::{AppHandle, Manager};

use crate::shell::state::AppState;

/// 主窗口是否处于 OS 认为的「获得焦点」态，**仅**由 `WindowEvent::Focused` 更新。
static SHELL_MAIN_WINDOW_FOCUSED_FROM_EVENT: AtomicBool = AtomicBool::new(false);

/// 进程启动至今是否收到过任何 `WindowEvent::Focused(true)`，仅用于诊断。
///
/// gate 判定本身不依赖它：`fse_windows::evaluate` 会同时看 `is_focused()` 与
/// Win32 前台归属，避免事件通道失灵时 gate 永久 Closed。
static SHELL_MAIN_WINDOW_EVER_FOCUSED: AtomicBool = AtomicBool::new(false);

/// 一次 gate 判定的全部输入，写进 trace 便于定位「为什么关」。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ShellGateDiagnostics {
    pub visible: bool,
    pub minimized: bool,
    pub focused_from_event: bool,
    pub is_focused: bool,
    pub win32_foreground: bool,
    pub ever_focused: bool,
}

/// 仅应由 `lib.rs` 里 `WindowEvent::Focused` 分支调用。
pub fn record_shell_main_window_focused_from_os_event(focused: bool) {
    if focused {
        SHELL_MAIN_WINDOW_EVER_FOCUSED.store(true, Ordering::Relaxed);
    }
    SHELL_MAIN_WINDOW_FOCUSED_FROM_EVENT.store(focused, Ordering::Relaxed);
}

/// 本进程是否曾经收到过 focus 事件（仅诊断用）。
pub fn shell_main_window_ever_focused() -> bool {
    SHELL_MAIN_WINDOW_EVER_FOCUSED.load(Ordering::Relaxed)
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

#[cfg(target_os = "windows")]
fn build_gate_hints(
    window: &tauri::WebviewWindow,
    focused_from_event: bool,
) -> (ShellWindowGateHints, ShellGateDiagnostics) {
    let evaluation = super::fse_windows::evaluate(window, focused_from_event);
    (
        evaluation.to_hints(),
        ShellGateDiagnostics {
            visible: evaluation.visible,
            minimized: evaluation.minimized,
            focused_from_event,
            is_focused: evaluation.is_focused,
            win32_foreground: evaluation.win32_foreground,
            ever_focused: shell_main_window_ever_focused(),
        },
    )
}

#[cfg(not(target_os = "windows"))]
fn build_gate_hints(
    window: &tauri::WebviewWindow,
    focused_from_event: bool,
) -> (ShellWindowGateHints, ShellGateDiagnostics) {
    (
        build_non_windows_gate_hints(focused_from_event),
        ShellGateDiagnostics {
            visible: window.is_visible().unwrap_or(true),
            minimized: window.is_minimized().unwrap_or(false),
            focused_from_event,
            is_focused: window.is_focused().unwrap_or(false),
            win32_foreground: false,
            ever_focused: shell_main_window_ever_focused(),
        },
    )
}

fn evaluate_shell_window(
    window: &tauri::WebviewWindow,
) -> (ShellWindowGateHints, ShellGateDiagnostics) {
    build_gate_hints(window, shell_main_window_focused_from_event())
}

pub fn current_shell_window_gate_hints(window: &tauri::WebviewWindow) -> ShellWindowGateHints {
    evaluate_shell_window(window).0
}

/// gate hints + 判定输入，供 trace 记录「为什么开/关」。
pub fn current_shell_window_gate_hints_with_diagnostics(
    window: &tauri::WebviewWindow,
) -> (ShellWindowGateHints, ShellGateDiagnostics) {
    evaluate_shell_window(window)
}

/// 主动请求主窗口前台归属（仅在没有焦点信号时作为兜底调用）。
pub fn request_shell_window_foreground(window: &tauri::WebviewWindow) -> bool {
    #[cfg(target_os = "windows")]
    {
        super::fse_windows::request_main_window_foreground(window)
    }
    #[cfg(not(target_os = "windows"))]
    {
        window.set_focus().is_ok()
    }
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
        record_shell_main_window_focused_from_os_event, shell_main_window_ever_focused,
        shell_main_window_focused_from_event,
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
        assert!(shell_main_window_ever_focused());

        clear_shell_main_window_focus_from_shell_action();

        assert!(!shell_main_window_focused_from_event());
        // ever_focused 是「曾经收到过」的历史事实，清标记不应回退它。
        assert!(shell_main_window_ever_focused());
    }
}
