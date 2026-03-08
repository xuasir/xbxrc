use tauri::Manager;
use tauri_plugin_keepawake::TauriPluginKeepawakeExt;

pub mod error;
pub mod event_bridge;
pub mod mods;
pub mod shell;

// 重导出以保持兼容性
pub use shell::state::{AppState, StartupFlagsState};

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_keepawake::init())
        .on_page_load(|webview, _payload| {
            let _ = webview.eval(shell::build_external_link_patch_script());
        })
        .on_window_event(|window, event| {
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
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            #[cfg(target_os = "macos")]
            {
                if let tauri::RunEvent::Reopen { .. } = event {
                    if let Some(window) = app_handle.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
            }

            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                tauri::async_runtime::block_on(shell::terminate(app_handle));
            }
        });
}
