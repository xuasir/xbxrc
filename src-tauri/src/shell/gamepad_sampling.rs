use tauri::{AppHandle, Manager};

use crate::mods::gamepad::input_gate;

use super::{
    plan_foreground_refresh_actions, plan_interactive_refresh_actions, AppState,
    ForegroundRefreshPlan,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShellSamplingRecoveryMode {
    /// 窗口事件带来的前台刷新：只在采样确实需要恢复时动设备。
    Foreground,
    /// 前端触屏提示：仅「用户动了一下」，绝不主动重开设备。
    Interactive,
    /// 系统级 resume（休眠/长时间挂起回来）：这是真正的恢复信号，强制恢复一次。
    AppResumed,
}

impl ShellSamplingRecoveryMode {
    fn trace_event(self) -> &'static str {
        match self {
            Self::Foreground => "shellForegroundRefresh",
            Self::Interactive => "shellInteractiveHint",
            Self::AppResumed => "shellAppResumedRefresh",
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
    record_gamepad_shell_trace(
        window,
        "windowFocused",
        serde_json::json!({
            "focused": focused,
            "windowLabel": window.label(),
        }),
    );
    // 失焦不改变 gate、也不转 BackgroundWarm：窗口仍在台前时手柄输入必须继续
    // 可用。只有隐藏 / 最小化才算「台后」，见 `handle_gamepad_window_resized`。
    if focused {
        refresh_gamepad_on_window_foreground(&window.app_handle(), "window-focused");
    }
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
            // 最小化是唯一会主动切「台后」的壳层动作。
            hint_gamepad_shell_background(window, "window-minimized");
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
    apply_shell_gamepad_sampling_recovery(
        app_handle,
        "app-resumed",
        ShellSamplingRecoveryMode::AppResumed,
    );
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

    let hints = input_gate::current_shell_window_gate_hints(&window);
    let plan = match mode {
        // 系统 resume 是真实的恢复信号（休眠/挂起回来），不依赖快照判定。
        ShellSamplingRecoveryMode::AppResumed => ForegroundRefreshPlan {
            should_resume_sampling: true,
            should_try_stalled_self_heal: true,
        },
        _ => match app_state.gamepad.get_runtime_snapshot() {
            Ok(snapshot) => match mode {
                ShellSamplingRecoveryMode::Foreground => {
                    plan_foreground_refresh_actions(&snapshot, &hints)
                }
                _ => plan_interactive_refresh_actions(&snapshot, &hints),
            },
            Err(error) => {
                app_state.runtime_trace.record_event(
                    "gamepad-shell",
                    "shellSamplingRecoverySnapshotUnavailable",
                    None,
                    serde_json::json!({
                        "reason": reason,
                        "mode": mode.trace_event(),
                        "error": error,
                    }),
                );
                return;
            }
        },
    };

    app_state.runtime_trace.record_event(
        "gamepad-shell",
        mode.trace_event(),
        None,
        serde_json::json!({
            "reason": reason,
            "windowLabel": window.label(),
            "streamPadForwarding": app_state.gamepad.stream_pad_forwarding(),
            "shellAppActive": hints.shell_app_active,
            "shouldResumeSampling": plan.should_resume_sampling,
            "shouldTryStalledSelfHeal": plan.should_try_stalled_self_heal,
        }),
    );

    if !plan.should_resume_sampling && !plan.should_try_stalled_self_heal {
        // 采样已经在正常工作：什么都不做。触屏、窗口事件都不允许打断正在跑的输入。
        if mode == ShellSamplingRecoveryMode::Interactive {
            app_state.runtime_trace.record_event(
                "gamepad-shell",
                "shellInteractiveHintSkipped",
                None,
                serde_json::json!({
                    "reason": reason,
                    "cause": "sampling-already-healthy",
                }),
            );
        }
        return;
    }

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
