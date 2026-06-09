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
            shell::handle_gamepad_page_loaded(
                &webview.window(),
                webview
                    .url()
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
            );
            let _ = webview.eval(shell::build_external_link_patch_script());
        })
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::Focused(focused) => {
                    shell::handle_gamepad_window_focus_changed(window, *focused);
                }
                tauri::WindowEvent::Resized(_) => shell::handle_gamepad_window_resized(window),
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
                shell::handle_gamepad_app_resumed(app_handle);
            }
            tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit => {
                tauri::async_runtime::block_on(shell::terminate(app_handle));
            }
            _ => {}
        }
    });
}
