//! Windows FSE（Gaming Full Screen Experience）检测与前台窗口 gate 判定。

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::OnceLock;

use ohmygamepad_sdl3::ShellWindowGateHints;
use tauri::{AppHandle, Manager, WebviewWindow};
use windows::core::PCSTR;
use windows::Win32::System::WindowsProgramming::IsApiSetImplemented;
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

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

pub fn foreground_hwnd_matches_main(main_hwnd: isize) -> bool {
    unsafe {
        let foreground = GetForegroundWindow();
        !foreground.0.is_null() && foreground.0 as isize == main_hwnd
    }
}

pub fn build_gate_hints(window: &WebviewWindow, focused_from_event: bool) -> ShellWindowGateHints {
    let fse_active = is_fse_active();
    let main_hwnd = try_main_window_hwnd(window);
    let foreground_hwnd_matches_main = main_hwnd.map(foreground_hwnd_matches_main).unwrap_or(false);

    let shell_app_active = if fse_active {
        foreground_hwnd_matches_main
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
}
