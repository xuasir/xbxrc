//! Windows 主窗口 gate hints 构建。
//!
//! 简化版：仅以窗口可见性 + 最小化状态 + Tauri focus 事件作为 shell_app_active 判定依据。
//! 不再依赖 Gaming Experience (FSE) API 或 Win32 前台 HWND 归属检查，
//! 避免全屏/FSE 下触屏软键盘抢前台导致 input gate 异常关闭的问题。

use ohmygamepad_sdl3::ShellWindowGateHints;
use tauri::WebviewWindow;

/// Windows 下统一用窗口可见性 + 最小化 + Tauri focus 事件判定 shell_app_active。
///
/// 不再区分 FSE / 全屏 / 窗口化模式 — 与非 Windows 平台保持同等简单度。
pub fn build_gate_hints(window: &WebviewWindow, focused_from_event: bool) -> ShellWindowGateHints {
    let visible = window.is_visible().unwrap_or(false);
    let minimized = window.is_minimized().unwrap_or(false);
    ShellWindowGateHints {
        shell_app_active: visible && !minimized && focused_from_event,
    }
}
