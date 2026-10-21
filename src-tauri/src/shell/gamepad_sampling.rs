use tauri::{AppHandle, Manager};

use crate::mods::gamepad::input_gate;

use super::{plan_foreground_refresh_actions, AppState, ForegroundRefreshPlan};

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

    refresh_gamepad_on_window_foreground(&window.app_handle(), "page-load");
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
