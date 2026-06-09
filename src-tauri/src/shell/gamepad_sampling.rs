#[cfg(target_os = "windows")]
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Manager};
#[cfg(target_os = "windows")]
use tokio::time::{sleep, Duration};

#[cfg(target_os = "windows")]
use crate::mods;
use crate::mods::gamepad::input_gate;

#[cfg(target_os = "windows")]
use super::trace_gamepad_runtime_snapshot;
use super::{plan_foreground_refresh_actions, AppState, ForegroundRefreshPlan};

#[cfg(target_os = "windows")]
static GAMEPAD_COLD_START_SDL_NUDGE_TASK_SCHEDULED: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "windows")]
static GAMEPAD_FSE_GATE_FALLBACK_SCHEDULED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShellSamplingRecoveryMode {
    Foreground,
    Interactive,
}

impl ShellSamplingRecoveryMode {
    fn trace_event(self) -> &'static str {
        match self {
            Self::Foreground => "shellForegroundRefresh",
            Self::Interactive => "shellInteractiveHint",
        }
    }
}

fn record_gamepad_shell_trace(window: &tauri::Window, event: &str, payload: serde_json::Value) {
    let Some(app_state) = window.try_state::<AppState>() else {
        return;
    };
    app_state
        .runtime_trace
        .record_event("gamepad-shell", event, None, payload);
}

fn hint_gamepad_shell_background(window: &tauri::Window, reason: &str) {
    use ohmygamepad_protocol::OhMyGamepadSamplingLifecycleDto;

    let Some(app_state) = window.try_state::<AppState>() else {
        return;
    };
    record_gamepad_shell_trace(
        window,
        "shellBackgroundHint",
        serde_json::json!({
            "reason": reason,
            "windowLabel": window.label(),
        }),
    );
    if let Err(error) = app_state
        .gamepad
        .set_sampling_lifecycle(OhMyGamepadSamplingLifecycleDto::BackgroundWarm)
    {
        log::warn!(
            "Failed to set gamepad sampling lifecycle BackgroundWarm reason={} error={}",
            reason,
            error
        );
    }
    input_gate::sync_gamepad_input_gate(&window.app_handle());
}

pub fn handle_gamepad_page_loaded(window: &tauri::Window, url: String) {
    record_gamepad_shell_trace(
        window,
        "pageLoad",
        serde_json::json!({
            "windowLabel": window.label(),
            "url": url,
        }),
    );

    let visible = window.is_visible().unwrap_or(false);
    let minimized = window.is_minimized().unwrap_or(false);
    refresh_gamepad_on_window_foreground(&window.app_handle(), "page-load");
    if visible && !minimized {
        #[cfg(target_os = "windows")]
        schedule_gamepad_cold_start_sdl_binding_nudge(&window.app_handle());
        #[cfg(target_os = "windows")]
        schedule_gamepad_fse_gate_fallback_nudge(&window.app_handle());
    }
}

pub fn handle_gamepad_window_focus_changed(window: &tauri::Window, focused: bool) {
    input_gate::record_shell_main_window_focused_from_os_event(focused);
    record_gamepad_shell_trace(
        window,
        "windowFocused",
        serde_json::json!({
            "focused": focused,
            "windowLabel": window.label(),
        }),
    );
    input_gate::sync_gamepad_input_gate(&window.app_handle());
    if focused {
        refresh_gamepad_on_window_foreground(&window.app_handle(), "window-focused");
        return;
    }

    let shell_still_active = window
        .app_handle()
        .get_webview_window("main")
        .map(|main_window| {
            let hints = input_gate::current_shell_window_gate_hints(&main_window);
            #[cfg(target_os = "windows")]
            record_gamepad_shell_trace(
                window,
                "windowUnfocusedGateEvaluated",
                serde_json::json!({
                    "windowLabel": window.label(),
                    "shellAppActive": hints.shell_app_active,
                    "usesWin32ForegroundGate":
                        mods::gamepad::fse_windows::uses_win32_foreground_gate(&main_window),
                    "isFseActive": mods::gamepad::fse_windows::is_fse_active(),
                    "isFullscreen": main_window.is_fullscreen().unwrap_or(false),
                }),
            );
            hints.shell_app_active
        })
        .unwrap_or(false);
    if shell_still_active {
        record_gamepad_shell_trace(
            window,
            "windowUnfocusedShellGateStillActive",
            serde_json::json!({
                "windowLabel": window.label(),
            }),
        );
        refresh_gamepad_on_window_foreground(
            &window.app_handle(),
            "window-unfocused-shell-gate-still-active",
        );
        return;
    }

    hint_gamepad_shell_background(window, "window-unfocused");
}

pub fn handle_gamepad_window_resized(window: &tauri::Window) {
    match window.is_minimized() {
        Ok(false) => {
            record_gamepad_shell_trace(
                window,
                "windowResizedRestored",
                serde_json::json!({
                    "windowLabel": window.label(),
                    "minimized": false,
                }),
            );
            refresh_gamepad_on_window_foreground(
                &window.app_handle(),
                "window-restored-from-minimized",
            );
        }
        Ok(true) => {
            record_gamepad_shell_trace(
                window,
                "windowResizedMinimized",
                serde_json::json!({
                    "windowLabel": window.label(),
                    "minimized": true,
                }),
            );
            input_gate::sync_gamepad_input_gate(&window.app_handle());
        }
        Err(error) => {
            log::warn!("Failed to inspect window minimized state: {}", error);
        }
    }
}

pub fn handle_gamepad_app_resumed(app_handle: &AppHandle) {
    if let Some(app_state) = app_handle.try_state::<AppState>() {
        app_state.runtime_trace.record_event(
            "gamepad-shell",
            "appResumed",
            None,
            serde_json::json!({
                "source": "runEvent",
            }),
        );
    }
    hint_gamepad_shell_interactive(app_handle, "app-resumed");
}

#[cfg(target_os = "windows")]
fn request_main_window_focus_for_input_stack(app: &AppHandle, reason: &str) {
    let Some(window) = app.get_webview_window("main") else {
        log::warn!(
            "request_main_window_focus_for_input_stack skipped reason={} (main window missing)",
            reason
        );
        return;
    };
    let _ = window.show();
    #[cfg(target_os = "windows")]
    let used_win32_foreground =
        super::win_foreground::try_force_webview_foreground_win32(&window, reason);
    #[cfg(not(target_os = "windows"))]
    let used_win32_foreground = false;
    if let Err(error) = window.set_focus() {
        log::warn!(
            "request_main_window_focus_for_input_stack set_focus failed reason={} error={}",
            reason,
            error
        );
    }
    if let Some(app_state) = window.try_state::<AppState>() {
        app_state.runtime_trace.record_event(
            "gamepad-shell",
            "mainWindowFocusRequestedForInputRecovery",
            None,
            serde_json::json!({
                "reason": reason,
                "windowLabel": window.label(),
                "usedWin32Foreground": used_win32_foreground,
            }),
        );
    }
}

pub fn refresh_gamepad_on_window_foreground(app: &AppHandle, reason: &str) {
    apply_shell_gamepad_sampling_recovery(app, reason, ShellSamplingRecoveryMode::Foreground);
}

pub fn hint_gamepad_shell_interactive(app: &AppHandle, reason: &str) {
    apply_shell_gamepad_sampling_recovery(app, reason, ShellSamplingRecoveryMode::Interactive);
}

fn apply_shell_gamepad_sampling_recovery(
    app: &AppHandle,
    reason: &str,
    mode: ShellSamplingRecoveryMode,
) {
    input_gate::sync_gamepad_input_gate(app);
    let Some(window) = app.get_webview_window("main") else {
        if mode == ShellSamplingRecoveryMode::Interactive {
            log::warn!(
                "hint_gamepad_shell_interactive skipped reason={} (main window missing)",
                reason
            );
        }
        return;
    };
    let Some(app_state) = window.try_state::<AppState>() else {
        if mode == ShellSamplingRecoveryMode::Interactive {
            log::warn!(
                "hint_gamepad_shell_interactive skipped reason={} (AppState not ready)",
                reason
            );
        }
        return;
    };
    app_state.runtime_trace.record_event(
        "gamepad-shell",
        mode.trace_event(),
        None,
        serde_json::json!({
            "reason": reason,
            "windowLabel": window.label(),
            "streamPadForwarding": app_state.gamepad.stream_pad_forwarding(),
        }),
    );

    let plan = match mode {
        ShellSamplingRecoveryMode::Foreground => {
            let Ok(snapshot) = app_state.gamepad.get_runtime_snapshot() else {
                return;
            };
            plan_foreground_refresh_actions(
                &snapshot,
                &input_gate::current_shell_window_gate_hints(&window),
            )
        }
        ShellSamplingRecoveryMode::Interactive => ForegroundRefreshPlan {
            should_resume_sampling: true,
            should_try_stalled_self_heal: true,
        },
    };

    if plan.should_resume_sampling {
        if let Err(error) = app_state.gamepad.resume_shell_sampling() {
            log::warn!(
                "gamepad shell sampling resume failed mode={:?} reason={} error={}",
                mode,
                reason,
                error
            );
        }
    }
    if plan.should_try_stalled_self_heal {
        if let Err(error) = app_state.gamepad.try_stalled_sampling_self_heal() {
            log::warn!(
                "gamepad stalled self-heal failed mode={:?} reason={} error={}",
                mode,
                reason,
                error
            );
        }
    }
}

#[cfg(target_os = "windows")]
pub(super) fn schedule_gamepad_cold_start_sdl_binding_nudge(app: &AppHandle) {
    if GAMEPAD_COLD_START_SDL_NUDGE_TASK_SCHEDULED.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        const PHASE_DELAYS_MS: [(u32, u64); 3] = [(0, 150), (1, 2850), (2, 5000)];
        let mut config_checked = false;

        for (phase_index, delay_ms) in PHASE_DELAYS_MS {
            sleep(Duration::from_millis(delay_ms)).await;

            let Some(state) = app.try_state::<AppState>() else {
                log::warn!("gamepad_cold_start_sdl_binding_nudge skipped (AppState not ready)");
                return;
            };
            if !config_checked {
                let enabled = match state
                    .config
                    .get_by_keys(&[String::from("gamepad_cold_start_sdl_binding_nudge")])
                {
                    Ok(value) => value
                        .get("gamepad_cold_start_sdl_binding_nudge")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true),
                    Err(_) => true,
                };
                if !enabled {
                    log::info!("gamepad_cold_start_sdl_binding_nudge disabled by config");
                    return;
                }
                config_checked = true;
            }

            state.runtime_trace.record_event(
                "gamepad-shell",
                "gamepadColdStartSdlBindingNudgeRun",
                None,
                serde_json::json!({
                    "phase": "begin",
                    "phaseIndex": phase_index,
                }),
            );

            if phase_index == 0 {
                request_main_window_focus_for_input_stack(&app, "cold-start-sdl-binding-nudge");
                input_gate::sync_gamepad_input_gate(&app);
                if let Err(error) = state.gamepad.set_sampling_lifecycle(
                    ohmygamepad_protocol::OhMyGamepadSamplingLifecycleDto::BackgroundWarm,
                ) {
                    log::warn!(
                        "gamepad_cold_start_sdl_binding_nudge BackgroundWarm failed error={}",
                        error
                    );
                }

                sleep(Duration::from_millis(80)).await;

                let Some(state) = app.try_state::<AppState>() else {
                    log::warn!("gamepad_cold_start_sdl_binding_nudge mid-run lost AppState");
                    return;
                };

                if let Err(error) = state.gamepad.set_sampling_lifecycle(
                    ohmygamepad_protocol::OhMyGamepadSamplingLifecycleDto::Active,
                ) {
                    log::warn!(
                        "gamepad_cold_start_sdl_binding_nudge Active failed error={}",
                        error
                    );
                }

                refresh_gamepad_on_window_foreground(&app, "cold-start-sdl-binding-nudge");
            }

            if let Ok(snapshot) = state.gamepad.get_runtime_snapshot() {
                trace_gamepad_runtime_snapshot(
                    &state.runtime_trace,
                    "gamepadColdStartSdlBindingNudgeCompleted",
                    &snapshot,
                    state.gamepad.stream_pad_forwarding(),
                    serde_json::json!({
                        "phase": "done",
                        "phaseIndex": phase_index,
                    }),
                );
            }
        }
    });
}

#[cfg(target_os = "windows")]
pub(super) fn schedule_gamepad_fse_gate_fallback_nudge(app: &AppHandle) {
    if GAMEPAD_FSE_GATE_FALLBACK_SCHEDULED.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        sleep(Duration::from_secs(4)).await;
        let Some(state) = app.try_state::<AppState>() else {
            return;
        };
        let enabled = match state
            .config
            .get_by_keys(&[String::from("gamepad_fse_gate_fallback_nudge")])
        {
            Ok(value) => value
                .get("gamepad_fse_gate_fallback_nudge")
                .and_then(|entry| entry.as_bool())
                .unwrap_or(false),
            Err(_) => false,
        };
        if !enabled {
            return;
        }
        let Some(window) = app.get_webview_window("main") else {
            return;
        };
        if !mods::gamepad::fse_windows::uses_win32_foreground_gate(&window) {
            return;
        }
        let Ok(snapshot) = state.gamepad.get_runtime_snapshot() else {
            return;
        };
        if snapshot.input_gate != ohmygamepad_protocol::OhMyGamepadInputGateModeDto::Closed {
            return;
        }
        if snapshot.slots.is_empty() {
            return;
        }
        state.runtime_trace.record_event(
            "gamepad-shell",
            "gamepadFseGateFallbackNudge",
            None,
            serde_json::json!({
                "inputGateReason": snapshot.input_gate_reason,
                "slotCount": snapshot.slots.len(),
            }),
        );
        hint_gamepad_shell_interactive(&app, "fse-gate-fallback-nudge");
    });
}
