//! Windows：用 user32 前台 API 强制主窗口获得焦点，补 Tauri `set_focus` 在部分环境（SDL/虚拟手柄冷启动）下力度不足的问题。

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tauri::WebviewWindow;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId,
    IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE,
};

/// 返回是否拿到了 Win32 `HWND` 并执行了置前序列（不保证 `SetForegroundWindow` 一定成功：受系统前台锁限制）。
pub fn try_force_webview_foreground_win32(window: &WebviewWindow, reason: &str) -> bool {
    let hwnd_isize = match window.window_handle() {
        Ok(handle) => match handle.as_raw() {
            RawWindowHandle::Win32(w) => w.hwnd.get(),
            _ => {
                log::warn!(
                    "win_foreground unsupported raw handle reason={} (expected Win32)",
                    reason
                );
                return false;
            }
        },
        Err(error) => {
            log::warn!(
                "win_foreground window_handle failed reason={} error={}",
                reason,
                error
            );
            return false;
        }
    };

    let hwnd = HWND(hwnd_isize as *mut _);
    if hwnd.0.is_null() {
        log::warn!("win_foreground null hwnd reason={}", reason);
        return false;
    }

    unsafe {
        // ASFW_ANY：尽量放宽前台锁（仍可能被系统拒绝）。
        let _ = AllowSetForegroundWindow(u32::MAX);

        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }

        let foreground = GetForegroundWindow();
        let current_tid = GetCurrentThreadId();

        if !foreground.0.is_null() {
            let fg_tid = GetWindowThreadProcessId(foreground, None);
            if fg_tid != 0 && fg_tid != current_tid {
                let _ = AttachThreadInput(fg_tid, current_tid, true);
                let ok = SetForegroundWindow(hwnd).as_bool();
                let _ = BringWindowToTop(hwnd);
                let _ = AttachThreadInput(fg_tid, current_tid, false);
                if !ok {
                    log::warn!(
                        "win_foreground SetForegroundWindow returned false reason={}",
                        reason
                    );
                }
                return true;
            }
        }

        let ok = SetForegroundWindow(hwnd).as_bool();
        let _ = BringWindowToTop(hwnd);
        if !ok {
            log::warn!(
                "win_foreground SetForegroundWindow returned false (no attach path) reason={}",
                reason
            );
        }
    }

    true
}
