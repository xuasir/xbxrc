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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,xbxrc_lib::mods::rpc=warn"),
    )
    .init();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_keepawake::init())
        .on_page_load(|webview, _payload| {
            let _ = webview.eval(shell::build_external_link_patch_script());
        })
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::Focused(focused) => {
                    let app_state = window.state::<shell::state::AppState>();
                    if let Err(e) = app_state.gamepad.set_suspended(!focused) {
                        log::warn!("Failed to toggle gamepad suspension on focus change: {}", e);
                    }
                }
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
            tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit => {
                tauri::async_runtime::block_on(shell::terminate(app_handle));
            }
            _ => {}
        }
    });
}
