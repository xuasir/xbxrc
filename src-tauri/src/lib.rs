#![recursion_limit = "256"]

use tauri::Manager;

pub mod error;
pub mod event_bridge;
pub mod mods;
pub mod settings_store;
pub mod shell;

// 重导出以保持兼容性
pub use shell::state::{AppState, StartupFlagsState};

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

fn install_rustls_crypto_provider() {
    // reqwest 会启用 aws-lc-rs，而 webrtc/dtls 会启用 ring。
    // rustls 0.23 在 provider 歧义时会 panic，所以这里在进程启动早期显式选定一次。
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

fn record_gamepad_shell_trace(window: &tauri::Window, event: &str, payload: serde_json::Value) {
    let app_state = window.state::<shell::state::AppState>();
    app_state
        .runtime_trace
        .record_event("gamepad-shell", event, None, payload);
}

fn hint_gamepad_shell_background(window: &tauri::Window, reason: &str) {
    use ohmygamepad_protocol::OhMyGamepadSamplingLifecycleDto;

    let app_state = window.state::<shell::state::AppState>();
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
    mods::gamepad::input_gate::sync_gamepad_input_gate(&window.app_handle());
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    install_rustls_crypto_provider();

    let mut logger = env_logger::Builder::from_env(
        // 终端默认只保留 warning 及以上，避免第三方 WebRTC crate 持续刷屏；
        // 需要更细粒度调试时，仍可通过 RUST_LOG 显式放开。
        env_logger::Env::default().default_filter_or("warn"),
    );
    logger.filter_module("webrtc_srtp::session", log::LevelFilter::Warn);
    logger.init();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_keepawake::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .on_page_load(|webview, _payload| {
            if let Some(app_state) = webview.window().try_state::<shell::state::AppState>() {
                app_state.runtime_trace.record_event(
                    "gamepad-shell",
                    "pageLoad",
                    None,
                    serde_json::json!({
                        "windowLabel": webview.window().label(),
                        "url": webview.url().map(|value| value.to_string()).unwrap_or_default(),
                    }),
                );
            }
            let visible = webview.window().is_visible().unwrap_or(false);
            let minimized = webview.window().is_minimized().unwrap_or(false);
            shell::refresh_gamepad_on_window_foreground(
                &webview.window().app_handle(),
                "page-load",
            );
            if visible && !minimized {
                #[cfg(target_os = "windows")]
                shell::schedule_gamepad_cold_start_sdl_binding_nudge(
                    &webview.window().app_handle(),
                );
                #[cfg(target_os = "windows")]
                shell::schedule_gamepad_fse_gate_fallback_nudge(&webview.window().app_handle());
            }
            let _ = webview.eval(shell::build_external_link_patch_script());
        })
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::Focused(focused) => {
                    mods::gamepad::input_gate::record_shell_main_window_focused_from_os_event(
                        *focused,
                    );
                    record_gamepad_shell_trace(
                        window,
                        "windowFocused",
                        serde_json::json!({
                            "focused": focused,
                            "windowLabel": window.label(),
                        }),
                    );
                    mods::gamepad::input_gate::sync_gamepad_input_gate(&window.app_handle());
                    if *focused {
                        shell::refresh_gamepad_on_window_foreground(
                            &window.app_handle(),
                            "window-focused",
                        );
                    } else {
                        let shell_still_active = window
                            .app_handle()
                            .get_webview_window("main")
                            .map(|main_window| {
                                let hints =
                                    mods::gamepad::input_gate::current_shell_window_gate_hints(
                                        &main_window,
                                    );
                                #[cfg(target_os = "windows")]
                                record_gamepad_shell_trace(
                                    window,
                                    "windowUnfocusedGateEvaluated",
                                    serde_json::json!({
                                        "windowLabel": window.label(),
                                        "shellAppActive": hints.shell_app_active,
                                        "usesWin32ForegroundGate":
                                            mods::gamepad::fse_windows::uses_win32_foreground_gate(
                                                &main_window,
                                            ),
                                        "isFseActive":
                                            mods::gamepad::fse_windows::is_fse_active(),
                                        "isFullscreen": main_window
                                            .is_fullscreen()
                                            .unwrap_or(false),
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
                        } else {
                            hint_gamepad_shell_background(window, "window-unfocused");
                        }
                    }
                }
                tauri::WindowEvent::Resized(_) => match window.is_minimized() {
                    Ok(false) => {
                        record_gamepad_shell_trace(
                            window,
                            "windowResizedRestored",
                            serde_json::json!({
                                "windowLabel": window.label(),
                                "minimized": false,
                            }),
                        );
                        shell::refresh_gamepad_on_window_foreground(
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
                        mods::gamepad::input_gate::sync_gamepad_input_gate(&window.app_handle());
                    }
                    Err(e) => {
                        log::warn!("Failed to inspect window minimized state: {}", e);
                    }
                },
                _ => {}
            }

            #[cfg(target_os = "macos")]
            shell::handle_macos_window_event(window, event);
        })
        .setup(|app| {
            // 将复杂的逻辑委托给 shell 模块
            if let Err(e) = tauri::async_runtime::block_on(shell::init_services(app)) {
                log::error!("Failed to initialize services: {:?}", e);
                return Err(e.into());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![greet, shell::rpc::rpc_invoke])
        .build(tauri::generate_context!());

    let app = match app {
        Ok(app) => app,
        Err(error) => {
            log::error!("Failed to build tauri application: {}", error);
            return;
        }
    };

    app.run(|app_handle, event| {
        #[cfg(target_os = "macos")]
        {
            if let tauri::RunEvent::Reopen { .. } = event {
                if let Some(window) = app_handle.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        }

        match event {
            tauri::RunEvent::Resumed => {
                let app_state = app_handle.state::<shell::state::AppState>();
                app_state.runtime_trace.record_event(
                    "gamepad-shell",
                    "appResumed",
                    None,
                    serde_json::json!({
                        "source": "runEvent",
                    }),
                );
                use ohmygamepad_protocol::OhMyGamepadSamplingLifecycleDto;
                if let Err(error) = app_state
                    .gamepad
                    .set_sampling_lifecycle(OhMyGamepadSamplingLifecycleDto::Active)
                {
                    log::warn!(
                        "Failed to set gamepad lifecycle on app resume error={}",
                        error
                    );
                }
                if let Err(error) = app_state.gamepad.try_stalled_sampling_self_heal() {
                    log::warn!(
                        "Failed to try stalled gamepad self-heal on app resume error={}",
                        error
                    );
                }
                mods::gamepad::input_gate::sync_gamepad_input_gate(app_handle);
            }
            tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit => {
                tauri::async_runtime::block_on(shell::terminate(app_handle));
            }
            _ => {}
        }
    });
}
