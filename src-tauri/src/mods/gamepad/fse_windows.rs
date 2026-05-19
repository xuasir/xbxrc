//! Windows FSE（Gaming Full Screen Experience）检测与前台窗口 gate 判定。
//!
//! GDK：FSE 或 Tauri 全屏下 gate 以 Win32 前台 HWND 归属为准；窗口化非 FSE 仍用 Tauri focus 事件。

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::OnceLock;

use ohmygamepad_sdl3::ShellWindowGateHints;
use tauri::{AppHandle, Manager, WebviewWindow};
use windows::core::PCSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::WindowsProgramming::IsApiSetImplemented;
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetForegroundWindow, GetWindowThreadProcessId, IsChild, GA_ROOT,
};

use crate::mods::gamepad::input_gate::sync_gamepad_input_gate;
use crate::shell::refresh_gamepad_on_window_foreground;
use crate::shell::win_hwnd::try_main_window_hwnd;

static FSE_ACTIVE: AtomicBool = AtomicBool::new(false);
static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();
static REGISTRATION_TOKEN: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

const GAMING_EXPERIENCE_API_SET: &str = "api-ms-win-gaming-experience-l1-1-0";
const GAMING_EXPERIENCE_API_SET_CSTR: &[u8] = b"api-ms-win-gaming-experience-l1-1-0\0";

type IsFseActiveFn = unsafe extern "system" fn() -> i32;
type RegisterFseChangeFn = unsafe extern "system" fn(
    callback: Option<unsafe extern "system" fn(*mut c_void)>,
    context: *mut c_void,
    registration: *mut *mut c_void,
) -> i32;
type UnregisterFseChangeFn = unsafe extern "system" fn(registration: *mut c_void) -> i32;

struct FseApiTable {
    is_active: IsFseActiveFn,
    register_change: RegisterFseChangeFn,
    unregister_change: UnregisterFseChangeFn,
}

static FSE_APIS: OnceLock<Option<FseApiTable>> = OnceLock::new();

fn gaming_experience_api_set_available() -> bool {
    unsafe { IsApiSetImplemented(PCSTR(GAMING_EXPERIENCE_API_SET_CSTR.as_ptr())).as_bool() }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FseApiLoadError {
    ApiSetUnavailable,
    LibraryUnavailable,
    SymbolUnavailable,
}

fn load_fse_apis() -> Result<FseApiTable, FseApiLoadError> {
    if !gaming_experience_api_set_available() {
        return Err(FseApiLoadError::ApiSetUnavailable);
    }
    unsafe {
        let lib = libloading::Library::new("GamingExperience.dll")
            .map_err(|_| FseApiLoadError::LibraryUnavailable)?;
        let is_active = *lib
            .get::<IsFseActiveFn>(b"IsGamingFullScreenExperienceActive\0")
            .map_err(|_| FseApiLoadError::SymbolUnavailable)?;
        let register_change = *lib
            .get::<RegisterFseChangeFn>(b"RegisterGamingFullScreenExperienceChangeNotification\0")
            .map_err(|_| FseApiLoadError::SymbolUnavailable)?;
        let unregister_change = *lib
            .get::<UnregisterFseChangeFn>(
                b"UnregisterGamingFullScreenExperienceChangeNotification\0",
            )
            .map_err(|_| FseApiLoadError::SymbolUnavailable)?;
        std::mem::forget(lib);
        Ok(FseApiTable {
            is_active,
            register_change,
            unregister_change,
        })
    }
}

fn refresh_fse_active_flag() {
    let Some(apis) = FSE_APIS.get().and_then(|entry| entry.as_ref()) else {
        return;
    };
    let active = unsafe { (apis.is_active)() != 0 };
    FSE_ACTIVE.store(active, Ordering::Relaxed);
}

extern "system" fn fse_change_callback(_context: *mut c_void) {
    refresh_fse_active_flag();
    if let Some(app) = APP_HANDLE.get().cloned() {
        tauri::async_runtime::spawn(async move {
            sync_gamepad_input_gate(&app);
            refresh_gamepad_on_window_foreground(&app, "fse-change");
        });
    }
}

pub fn is_fse_active() -> bool {
    FSE_ACTIVE.load(Ordering::Relaxed)
}

pub fn init_fse_monitor(app: &AppHandle) {
    let _ = APP_HANDLE.set(app.clone());
    let apis = match load_fse_apis() {
        Ok(apis) => Some(apis),
        Err(FseApiLoadError::ApiSetUnavailable) => {
            log::info!(
                "fse_windows Gaming Experience API set unavailable ({}); using desktop foreground path",
                GAMING_EXPERIENCE_API_SET
            );
            None
        }
        Err(FseApiLoadError::LibraryUnavailable) => {
            log::info!(
                "fse_windows GamingExperience.dll unavailable; using desktop foreground path"
            );
            None
        }
        Err(FseApiLoadError::SymbolUnavailable) => {
            log::warn!("fse_windows GamingExperience.dll missing required symbols; using desktop foreground path");
            None
        }
    };
    let _ = FSE_APIS.set(apis);
    refresh_fse_active_flag();

    if let Some(apis) = FSE_APIS.get().and_then(|entry| entry.as_ref()) {
        let mut token: *mut c_void = std::ptr::null_mut();
        let hr = unsafe {
            (apis.register_change)(Some(fse_change_callback), std::ptr::null_mut(), &mut token)
        };
        if hr < 0 {
            log::warn!(
                "fse_windows RegisterGamingFullScreenExperienceChangeNotification failed hr={hr}"
            );
        } else {
            REGISTRATION_TOKEN.store(token, Ordering::Relaxed);
            log::info!(
                "fse_windows monitor registered active={}",
                FSE_ACTIVE.load(Ordering::Relaxed)
            );
        }
    }

    schedule_win32_foreground_gate_resync(app);
}

/// Win32 前台 gate 冷启动重同步（FSE 或 Tauri 全屏；change notification 可能尚未触发）。
fn schedule_win32_foreground_gate_resync(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        const PHASE_DELAYS_MS: [(u32, u64); 3] = [(0, 0), (1, 250), (2, 1000)];
        for (phase_index, delay_ms) in PHASE_DELAYS_MS {
            if delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
            let Some(window) = app.get_webview_window("main") else {
                return;
            };
            if !uses_win32_foreground_gate(&window) {
                continue;
            }
            sync_gamepad_input_gate(&app);
            refresh_gamepad_on_window_foreground(
                &app,
                &format!("win32-foreground-gate-resync-{phase_index}"),
            );
        }
    });
}

pub fn shutdown_fse_monitor() {
    let token = REGISTRATION_TOKEN.swap(std::ptr::null_mut(), Ordering::Relaxed);
    if token.is_null() {
        return;
    }

    let Some(apis) = FSE_APIS.get().and_then(|entry| entry.as_ref()) else {
        log::warn!("fse_windows unregister skipped (apis unavailable)");
        return;
    };

    let hr = unsafe { (apis.unregister_change)(token) };
    if hr < 0 {
        log::warn!("fse_windows unregister change notification failed hr={hr}");
    }
}

/// 前台 HWND 是否仍属于主窗口（含 WebView2 子窗口；触屏后常见前台落在子 HWND 上）。
fn foreground_hwnd_belongs_to_main(main_hwnd: isize, foreground_hwnd: isize) -> bool {
    if main_hwnd == foreground_hwnd {
        return true;
    }
    unsafe {
        let main = HWND(main_hwnd as *mut _);
        let foreground = HWND(foreground_hwnd as *mut _);
        if IsChild(main, foreground).as_bool() {
            return true;
        }
        let main_root = GetAncestor(main, GA_ROOT);
        let foreground_root = GetAncestor(foreground, GA_ROOT);
        !main_root.0.is_null() && main_root == foreground_root
    }
}

fn foreground_same_process_as_main(main_hwnd: isize, foreground_hwnd: isize) -> bool {
    unsafe {
        let mut foreground_pid = 0u32;
        let mut main_pid = 0u32;
        GetWindowThreadProcessId(HWND(foreground_hwnd as *mut _), Some(&mut foreground_pid));
        GetWindowThreadProcessId(HWND(main_hwnd as *mut _), Some(&mut main_pid));
        foreground_pid != 0 && foreground_pid == main_pid
    }
}

pub fn foreground_belongs_to_main(main_hwnd: isize) -> bool {
    unsafe {
        let foreground = GetForegroundWindow();
        if foreground.0.is_null() {
            return false;
        }
        let foreground_hwnd = foreground.0 as isize;
        if foreground_hwnd_belongs_to_main(main_hwnd, foreground_hwnd) {
            return true;
        }
        foreground_same_process_as_main(main_hwnd, foreground_hwnd)
    }
}

/// Gate 以 Win32 前台 HWND 为准（GDK）：FSE API 激活，或壳层处于 Tauri 全屏。
pub fn uses_win32_foreground_gate(window: &WebviewWindow) -> bool {
    is_fse_active() || window.is_fullscreen().unwrap_or(false)
}

/// Win32 前台 gate：窗口可见且前台仍归属本应用（不读 Tauri Focused，避免触屏假失焦）。
fn win32_foreground_shell_app_active(
    foreground_ok: bool,
    window_visible: bool,
    window_minimized: bool,
) -> bool {
    window_visible && !window_minimized && foreground_ok
}

pub fn build_gate_hints(window: &WebviewWindow, focused_from_event: bool) -> ShellWindowGateHints {
    if !uses_win32_foreground_gate(window) {
        return ShellWindowGateHints {
            shell_app_active: focused_from_event,
        };
    }

    let window_visible = window.is_visible().unwrap_or(false);
    let window_minimized = window.is_minimized().unwrap_or(false);
    let foreground_ok = try_main_window_hwnd(window)
        .map(foreground_belongs_to_main)
        .unwrap_or(false);

    ShellWindowGateHints {
        shell_app_active: win32_foreground_shell_app_active(
            foreground_ok,
            window_visible,
            window_minimized,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{FseApiLoadError, GAMING_EXPERIENCE_API_SET, GAMING_EXPERIENCE_API_SET_CSTR};

    #[test]
    fn gaming_experience_api_set_name_matches_microsoft_contract() {
        assert_eq!(
            GAMING_EXPERIENCE_API_SET,
            "api-ms-win-gaming-experience-l1-1-0"
        );
    }

    #[test]
    fn gaming_experience_api_set_name_is_nul_terminated_for_is_api_set_implemented() {
        assert_eq!(
            GAMING_EXPERIENCE_API_SET_CSTR,
            b"api-ms-win-gaming-experience-l1-1-0\0"
        );
    }

    #[test]
    fn api_set_unavailable_is_distinct_from_missing_library() {
        assert_ne!(
            FseApiLoadError::ApiSetUnavailable,
            FseApiLoadError::LibraryUnavailable
        );
    }

    #[test]
    fn foreground_hwnd_belongs_to_main_when_handles_match() {
        assert!(super::foreground_hwnd_belongs_to_main(0x100, 0x100));
    }

    #[test]
    fn foreground_hwnd_belongs_to_main_when_handles_differ_without_win32() {
        assert!(!super::foreground_hwnd_belongs_to_main(0x100, 0x200));
    }

    #[test]
    fn win32_foreground_gate_follows_foreground_and_window_visibility() {
        assert!(super::win32_foreground_shell_app_active(true, true, false));
        assert!(!super::win32_foreground_shell_app_active(
            false, true, false
        ));
        assert!(!super::win32_foreground_shell_app_active(
            true, false, false
        ));
        assert!(!super::win32_foreground_shell_app_active(true, true, true));
    }
}
