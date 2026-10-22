//! 主窗口前台状态 → ohmygamepad gate。
//!
//! 判定只有一个维度：**窗口是否在台前**（可见且未最小化）。
//!
//! 刻意**不**看焦点事件、也不看 Win32 前台窗口归属：全屏 / FSE / 掌机启动器
//! 拉起的路径上这两个信号都不可靠，一旦拿它当关闭条件，就会出现「手柄已连接、
//! 震动正常、输入全无」。gate 误关的代价是完全没有输入，误开的代价只是后台
//! 多收几个包 —— 所以这里只做最保守的判定，宁可多放行。

use ohmygamepad_sdl3::ShellWindowGateHints;
use tauri::{AppHandle, Manager};

use crate::shell::state::AppState;

/// 一次前台判定的输入与结论，写进 trace 便于定位「为什么被当成后台」。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ShellForegroundState {
    pub visible: bool,
    pub minimized: bool,
}

impl ShellForegroundState {
    /// 台前 = 窗口可见 + 未最小化。
    pub fn is_foreground(&self) -> bool {
        self.visible && !self.minimized
    }

    pub fn to_hints(&self) -> ShellWindowGateHints {
        ShellWindowGateHints {
            shell_app_active: self.is_foreground(),
        }
    }
}

/// 判定主窗口是否台前。查询失败时按台前处理，绝不因查询异常吞输入。
pub fn evaluate_shell_foreground(window: &tauri::WebviewWindow) -> ShellForegroundState {
    ShellForegroundState {
        visible: window.is_visible().unwrap_or(true),
        minimized: window.is_minimized().unwrap_or(false),
    }
}

pub fn current_shell_window_gate_hints(window: &tauri::WebviewWindow) -> ShellWindowGateHints {
    evaluate_shell_foreground(window).to_hints()
}

/// gate hints + 判定输入，供 trace 记录「为什么开/关」。
pub fn current_shell_window_gate_hints_with_state(
    window: &tauri::WebviewWindow,
) -> (ShellWindowGateHints, ShellForegroundState) {
    let state = evaluate_shell_foreground(window);
    (state.to_hints(), state)
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

#[cfg(test)]
mod tests {
    use super::ShellForegroundState;

    fn state(visible: bool, minimized: bool) -> ShellForegroundState {
        ShellForegroundState { visible, minimized }
    }

    #[test]
    fn visible_and_restored_is_foreground() {
        assert!(state(true, false).is_foreground());
        assert!(state(true, false).to_hints().shell_app_active);
    }

    #[test]
    fn invisible_or_minimized_is_background() {
        assert!(!state(false, false).is_foreground());
        assert!(!state(true, true).is_foreground());
        assert!(!state(false, true).is_foreground());
    }
}
