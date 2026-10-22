//! Windows 主窗口 gate hints 构建。
//!
//! gate 判定必须容忍「focus 事件丢失」，因此这里以**多信号取并集**的方式判定
//! `shell_app_active`，而不是只信任 `WindowEvent::Focused`。

use ohmygamepad_sdl3::ShellWindowGateHints;
use tauri::WebviewWindow;

/// 一次 gate 判定的完整输入，便于把「为什么关」写进 trace。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WindowsGateEvaluation {
    pub visible: bool,
    pub minimized: bool,
    pub focused_from_event: bool,
    pub is_focused: bool,
    pub win32_foreground: bool,
}

impl WindowsGateEvaluation {
    pub fn shell_app_active(&self) -> bool {
        self.visible
            && !self.minimized
            && (self.focused_from_event || self.is_focused || self.win32_foreground)
    }

    pub fn to_hints(&self) -> ShellWindowGateHints {
        ShellWindowGateHints {
            shell_app_active: self.shell_app_active(),
        }
    }
}

/// 收集判定输入。三个 focus 信号**取并集**，任一为真即认为拥有输入归属：
///
/// 1. `focused_from_event`：`WindowEvent::Focused` 粘性标记，正常路径主信号；
/// 2. `window.is_focused()`：Tauri 按 WM_SETFOCUS/WM_KILLFOCUS 维护的状态，
///    兜住「我们的 Focused 事件被漏发」的情况；
/// 3. Win32 前台归属：`GetForegroundWindow` 指向主窗口，或指向本进程任意窗口
///    （WebView2 / FSE 合成窗口是不同 HWND）。
///
/// 第 3 条是必需的：全屏 / FSE 冷启动与全屏切换时，focused 事件可能只送达一次
/// `false` 后再不回来，此时只靠前两条会让 gate 永久停在 Closed，表现为手柄
/// 已连接、震动可用、但输入完全无响应。
pub fn evaluate(window: &WebviewWindow, focused_from_event: bool) -> WindowsGateEvaluation {
    WindowsGateEvaluation {
        visible: window.is_visible().unwrap_or(true),
        minimized: window.is_minimized().unwrap_or(false),
        focused_from_event,
        is_focused: window.is_focused().unwrap_or(false),
        win32_foreground: win32_foreground_belongs_to(window),
    }
}

pub fn build_gate_hints(window: &WebviewWindow, focused_from_event: bool) -> ShellWindowGateHints {
    evaluate(window, focused_from_event).to_hints()
}

#[cfg(test)]
mod tests {
    use super::WindowsGateEvaluation;

    fn baseline() -> WindowsGateEvaluation {
        WindowsGateEvaluation {
            visible: true,
            minimized: false,
            focused_from_event: false,
            is_focused: false,
            win32_foreground: false,
        }
    }

    #[test]
    fn any_single_focus_signal_opens_gate() {
        for open in [
            WindowsGateEvaluation {
                focused_from_event: true,
                ..baseline()
            },
            WindowsGateEvaluation {
                is_focused: true,
                ..baseline()
            },
            // 关键回归用例：全屏切换后 focus 事件不再送达，只剩 Win32 前台归属。
            WindowsGateEvaluation {
                win32_foreground: true,
                ..baseline()
            },
        ] {
            assert!(open.shell_app_active(), "expected open for {open:?}");
        }
    }

    #[test]
    fn invisible_or_minimized_never_opens_gate() {
        assert!(!WindowsGateEvaluation {
            visible: false,
            focused_from_event: true,
            is_focused: true,
            win32_foreground: true,
            ..baseline()
        }
        .shell_app_active());
        assert!(!WindowsGateEvaluation {
            minimized: true,
            focused_from_event: true,
            is_focused: true,
            win32_foreground: true,
            ..baseline()
        }
        .shell_app_active());
    }

    #[test]
    fn all_signals_false_keeps_gate_closed() {
        assert!(!baseline().shell_app_active());
    }
}

/// 主动把主窗口拉回前台（`show` + `SetForegroundWindow`）。
///
/// 仅在「完全没有 focus 信号、Win32 前台也不属于本进程」时调用：全屏/掌机由
/// 外部启动器拉起时，窗口可能既收不到 Focused 事件、也拿不到前台，此时只能
/// 主动请求一次。已有焦点时调用无副作用。
pub fn request_main_window_foreground(window: &WebviewWindow) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow;

    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    let Ok(hwnd) = window.hwnd() else {
        return false;
    };
    if hwnd.0.is_null() {
        return false;
    }
    unsafe { SetForegroundWindow(windows::Win32::Foundation::HWND(hwnd.0)).as_bool() }
}

/// Win32 前台窗口是否归属本应用。
///
/// 只看两种归属：前台窗口 root 就是主窗口 HWND，或前台窗口与主窗口同进程。
/// 不读 Tauri focus 状态，因此不受事件丢失影响。
fn win32_foreground_belongs_to(window: &WebviewWindow) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetAncestor, GetForegroundWindow, GetWindowThreadProcessId, GA_ROOT,
    };

    // tauri 依赖的 windows 版本与当前 crate 不同，HWND 不能直接比较，统一转成裸指针。
    let Ok(hwnd) = window.hwnd() else {
        return false;
    };
    let hwnd_raw = hwnd.0 as isize;
    if hwnd_raw == 0 {
        return false;
    }
    unsafe {
        let foreground = GetForegroundWindow();
        if foreground.0.is_null() {
            return false;
        }
        if GetAncestor(foreground, GA_ROOT).0 as isize == hwnd_raw {
            return true;
        }
        let mut foreground_pid = 0u32;
        GetWindowThreadProcessId(foreground, Some(&mut foreground_pid as *mut u32));
        foreground_pid != 0 && foreground_pid == std::process::id()
    }
}
