//! 从 Tauri 主窗口解析 Win32 `HWND`（供 FSE foreground 判定）。

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tauri::WebviewWindow;

pub fn try_main_window_hwnd(window: &WebviewWindow) -> Option<isize> {
    let handle = window.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(win32) => {
            let hwnd = win32.hwnd.get();
            if hwnd == 0 {
                None
            } else {
                Some(hwnd)
            }
        }
        _ => None,
    }
}
