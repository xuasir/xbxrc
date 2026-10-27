#![recursion_limit = "256"]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex as StdMutex;

use tauri::Manager;

pub mod error;
pub mod event_bridge;
pub mod mods;
pub mod settings_store;
pub mod shell;

// 重导出以保持兼容性
pub use shell::state::{AppState, StartupFlagsState};

/// 与 `src-tauri/tauri.conf.json` 的 `identifier` 保持一致。
///
/// 启动日志要在 `AppHandle` 可用之前就能落盘，所以这里不能依赖 tauri 的路径解析。
const APP_IDENTIFIER: &str = "com.xuasir.xbxrc";

/// 单个启动日志文件的体积上限，超过后轮转为 `startup.prev.log`。
const STARTUP_LOG_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// 同时写 stderr 与磁盘的日志出口。
///
/// release 构建带 `windows_subsystem = "windows"`，没有控制台，stderr 会被丢弃；
/// 不落盘就等于启动失败时没有任何现场。
struct TeeWriter {
    file: StdMutex<std::fs::File>,
}

impl TeeWriter {
    fn open(path: &Path) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self {
            file: StdMutex::new(file),
        })
    }
}

impl Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Ok(mut file) = self.file.lock() {
            let _ = file.write_all(buf);
        }
        let _ = std::io::stderr().write_all(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Ok(mut file) = self.file.lock() {
            let _ = file.flush();
        }
        Ok(())
    }
}

fn startup_log_path() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))?;
    Some(base.join(APP_IDENTIFIER).join("logs").join("startup.log"))
}

fn rotate_startup_log(path: &Path) {
    let too_large = std::fs::metadata(path)
        .map(|meta| meta.len() > STARTUP_LOG_MAX_BYTES)
        .unwrap_or(false);
    if too_large {
        let _ = std::fs::rename(path, path.with_extension("prev.log"));
    }
}

fn init_logger() {
    let mut logger = env_logger::Builder::from_env(
        // 终端默认只保留 warning 及以上，避免第三方 WebRTC crate 持续刷屏；
        // 需要更细粒度调试时，仍可通过 RUST_LOG 显式放开。
        env_logger::Env::default().default_filter_or("warn"),
    );
    logger.filter_module("webrtc_srtp::session", log::LevelFilter::Warn);

    if let Some(path) = startup_log_path() {
        rotate_startup_log(&path);
        if let Ok(writer) = TeeWriter::open(&path) {
            logger.target(env_logger::Target::Pipe(Box::new(writer)));
        }
    }

    logger.init();
}

/// panic 不走 `log`，单独落盘，避免 release 下 backtrace 完全丢失。
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(path) = startup_log_path() {
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
            {
                let _ = writeln!(file, "[panic] {info}");
            }
        }
        default_hook(info);
    }));
}

/// 启动失败时给出可见反馈，而不是静默退出。
fn report_startup_failure(message: &str) {
    log::error!("Startup failed: {message}");
    let log_hint = startup_log_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "(启动日志不可用)".to_string());
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("xbxrc 启动失败")
        .set_description(format!("{message}\n\n启动日志：{log_hint}"))
        .show();
}

fn install_rustls_crypto_provider() {
    // reqwest 会启用 aws-lc-rs，而 webrtc/dtls 会启用 ring。
    // rustls 0.23 在 provider 歧义时会 panic，所以这里在进程启动早期显式选定一次。
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    install_rustls_crypto_provider();
    init_logger();
    install_panic_hook();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_keepawake::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
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
                report_startup_failure(&e.to_string());
                return Err(e.into());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![greet, shell::rpc::rpc_invoke])
        .build(tauri::generate_context!());

    let app = match app {
        Ok(app) => app,
        Err(error) => {
            report_startup_failure(&format!("Failed to build tauri application: {error}"));
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
