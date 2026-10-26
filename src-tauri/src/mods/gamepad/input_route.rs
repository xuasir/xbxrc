//! 主窗口前台状态 → ohmygamepad 输入路由（Blocked 分支）。
//!
//! 判定只有一个维度：**窗口是否最小化**（macOS 额外要求可见，因为收起走 hide）。
//!
//! 刻意**不**看焦点事件、Win32 前台归属，Windows 上也不看 `is_visible()`：
//! 全屏 / FSE / 掌机启动器拉起的路径上 `is_visible()` 会短暂或长期返回 false，
//! 拿它当关闭条件就正好是历史上反复出现的「启动没输入、碰一下屏幕也没输入」。
//! route 误判为 Blocked 的代价是完全没有输入，误放行的代价只是后台多收几个包 ——
//! 所以只认「用户亲手最小化」这一个硬信号，宁可多放行。

use ohmygamepad_sdl3::ShellWindowRouteHints;
use tauri::{AppHandle, Manager};

use crate::shell::state::AppState;

/// 一次前台判定的输入与结论，写进 trace 便于定位「为什么被当成后台」。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ShellForegroundState {
    pub visible: bool,
    pub minimized: bool,
}

impl ShellForegroundState {
    /// 台前 = 未最小化；macOS 额外要求可见（收起窗口是 `hide()`，不一定会最小化）。
    pub fn is_foreground(&self) -> bool {
        if self.minimized {
            return false;
        }
        !cfg!(target_os = "macos") || self.visible
    }

    pub fn to_hints(&self) -> ShellWindowRouteHints {
        ShellWindowRouteHints {
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

pub fn current_shell_window_route_hints(window: &tauri::WebviewWindow) -> ShellWindowRouteHints {
    evaluate_shell_foreground(window).to_hints()
}

/// route hints + 判定输入，供 trace 记录「为什么被当成后台」。
pub fn current_shell_window_route_hints_with_state(
    window: &tauri::WebviewWindow,
) -> (ShellWindowRouteHints, ShellForegroundState) {
    let state = evaluate_shell_foreground(window);
    (state.to_hints(), state)
}

pub fn sync_gamepad_input_route(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let Some(window) = app.get_webview_window("main") else {
        // 取不到窗口时保持上一次判定，**不要**写 `shell_app_active: false`：
        // FSE / 冷启动窗口尚未就绪就会走这条路径，一旦写成 Blocked 又没有任何
        // 后续事件来纠正，输入就永久死掉。宁可多放行，绝不静默关死。
        return;
    };

    state
        .gamepad
        .set_shell_window_route_hints(current_shell_window_route_hints(&window));
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
    fn minimized_is_background() {
        assert!(!state(true, true).is_foreground());
        assert!(!state(false, true).is_foreground());
    }

    /// FSE / 全屏下 `is_visible()` 会误报 false，Windows 上不能拿它关 route。
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn windows_ignores_invisible_so_fse_keeps_input() {
        assert!(state(false, false).is_foreground());
        assert!(state(false, false).to_hints().shell_app_active);
    }

    /// macOS 收起窗口走 hide()，不可见就是台后。
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_hidden_window_is_background() {
        assert!(!state(false, false).is_foreground());
    }
}
