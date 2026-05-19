//! Windows FSE（Gaming Full Screen Experience）检测与前台窗口 gate 判定。

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::OnceLock;

use ohmygamepad_sdl3::ShellWindowGateHints;
use tauri::{AppHandle, WebviewWindow};
use windows::core::PCSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::WindowsProgramming::IsApiSetImplemented;
use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, GetForegroundWindow, IsChild, GA_ROOT};

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

    let Some(apis) = FSE_APIS.get().and_then(|entry| entry.as_ref()) else {
        return;
    };

    let mut token: *mut c_void = std::ptr::null_mut();
    let hr = unsafe {
        (apis.register_change)(Some(fse_change_callback), std::ptr::null_mut(), &mut token)
    };
    if hr < 0 {
        log::warn!(
            "fse_windows RegisterGamingFullScreenExperienceChangeNotification failed hr={hr}"
        );
        return;
    }
    REGISTRATION_TOKEN.store(token, Ordering::Relaxed);
    log::info!(
        "fse_windows monitor registered active={}",
        FSE_ACTIVE.load(Ordering::Relaxed)
    );
    schedule_fse_cold_start_foreground_resync(app);
}

/// FSE 已在启动前激活时，change notification 不会触发；短时重读 foreground 并刷新 gate/采样。
fn schedule_fse_cold_start_foreground_resync(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        const PHASE_DELAYS_MS: [(u32, u64); 3] = [(0, 0), (1, 250), (2, 1000)];
        for (phase_index, delay_ms) in PHASE_DELAYS_MS {
            if delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
            if !is_fse_active() {
                continue;
            }
            sync_gamepad_input_gate(&app);
            refresh_gamepad_on_window_foreground(
                &app,
                &format!("fse-cold-start-foreground-resync-{phase_index}"),
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

pub fn foreground_belongs_to_main(main_hwnd: isize) -> bool {
    unsafe {
        let foreground = GetForegroundWindow();
        if foreground.0.is_null() {
            return false;
        }
        foreground_hwnd_belongs_to_main(main_hwnd, foreground.0 as isize)
    }
}

/// FSE 下 gate：Win32 前台 HWND 归属 **或** 主窗口仍聚焦（触屏后常见子 HWND 抢前台但 Tauri 仍报失焦）。
fn fse_shell_app_active(
    foreground_belongs_to_main: bool,
    focused_from_event: bool,
    window_visible: bool,
    window_minimized: bool,
) -> bool {
    foreground_belongs_to_main || (focused_from_event && window_visible && !window_minimized)
}

pub fn build_gate_hints(window: &WebviewWindow, focused_from_event: bool) -> ShellWindowGateHints {
    let fse_active = is_fse_active();
    let window_visible = window.is_visible().unwrap_or(false);
    let window_minimized = window.is_minimized().unwrap_or(false);
    let main_hwnd = try_main_window_hwnd(window);
    let foreground_ok = main_hwnd.map(foreground_belongs_to_main).unwrap_or(false);

    let shell_app_active = if fse_active {
        fse_shell_app_active(
            foreground_ok,
            focused_from_event,
            window_visible,
            window_minimized,
        )
    } else {
        focused_from_event
    };

    ShellWindowGateHints { shell_app_active }
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
    fn fse_gate_uses_foreground_or_focused_visible_window() {
        assert!(super::fse_shell_app_active(true, false, true, false));
        assert!(super::fse_shell_app_active(false, true, true, false));
        assert!(!super::fse_shell_app_active(false, false, true, false));
        assert!(!super::fse_shell_app_active(false, true, false, false));
        assert!(!super::fse_shell_app_active(false, true, true, true));
    }
}
