use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use tauri::{Emitter, Manager};
use tauri_plugin_keepawake::{KeepAwakeConfig, TauriPluginKeepawakeExt};
use tokio::sync::{Mutex, RwLock};
use xbxengine_app::{XbxEngineApp, XbxEngineWindowHost, XbxEngineWindowState};
use xbxengine_protocol::XbxEngineRuntimeEventDto;

pub mod event_bridge;
pub mod mods;
pub mod rpc;

#[derive(Clone, Debug)]
pub struct StartupFlagsState {
    pub fullscreen: bool,
    pub auto_connect: String,
}

pub struct AppState {
    pub engine: Arc<Mutex<XbxEngineApp>>,
    pub app_state: Arc<mods::app_state::AppStateService>,
    pub auth: Arc<RwLock<mods::auth::AuthService>>,
    pub config: Arc<RwLock<mods::config::ConfigService>>,
    pub data: Arc<RwLock<mods::data::DataService>>,
    pub streaming: Arc<mods::streaming::StreamingService>,
    pub xbxengine: Arc<mods::xbxengine::XbxEngineService>,
    pub gamepad: Arc<mods::gamepad::GamepadService>,
    pub startup_flags: Arc<RwLock<StartupFlagsState>>,
    pub is_quitting: Arc<AtomicBool>,
    pub last_runtime_event: Arc<StdMutex<Option<serde_json::Value>>>,
}

use xbxengine_app::NoopXbxEngineAppHostBridge;

struct PrintlnHostBridge {
    app_handle: tauri::AppHandle,
    state: XbxEngineWindowState,
    last_runtime_event: Arc<StdMutex<Option<serde_json::Value>>>,
}

impl XbxEngineWindowHost for PrintlnHostBridge {
    fn open_window(&mut self, title: &str) {
        self.state.title = title.to_string();
    }

    fn apply_event(&mut self, event: &XbxEngineRuntimeEventDto) {
        match event {
            XbxEngineRuntimeEventDto::RuntimePhaseChanged { phase } => {
                self.state.runtime_phase = Some(phase.clone());
            }
            XbxEngineRuntimeEventDto::TransportConnectionStateChanged { state } => {
                self.state.transport_state = Some(state.clone());
            }
            XbxEngineRuntimeEventDto::MediaSurfaceReady { surface_id } => {
                self.state.surface_id = Some(surface_id.clone());
            }
            XbxEngineRuntimeEventDto::MediaVideoReady { width, height } => {
                self.state.video_size = Some((*width, *height));
            }
            _ => {}
        }

        if let Some(payload) = event_bridge::map_xbxengine_runtime_event(event) {
            if let Ok(mut lock) = self.last_runtime_event.lock() {
                *lock = Some(payload.clone());
            }
            let _ = self.app_handle.emit(
                event_bridge::STREAMING_XBXENGINE_RUNTIME_EVENT_CHANNEL,
                payload,
            );
        }
    }

    fn snapshot(&self) -> XbxEngineWindowState {
        self.state.clone()
    }
}

fn parse_startup_flags() -> StartupFlagsState {
    let mut fullscreen = false;
    let mut auto_connect = String::new();

    for arg in std::env::args() {
        if arg.contains("--fullscreen") {
            fullscreen = true;
        }

        if let Some(value) = arg.strip_prefix("--auto-connect=") {
            auto_connect = value.trim().to_string();
        }
    }

    StartupFlagsState {
        fullscreen,
        auto_connect,
    }
}

fn build_external_link_patch_script() -> &'static str {
    r#"
      (() => {
        if (window.__XBXRC_EXTERNAL_LINK_PATCHED__) return;
        window.__XBXRC_EXTERNAL_LINK_PATCHED__ = true;

        const invoke = window.__TAURI_INTERNALS__?.invoke;
        const openExternal = (url) => {
          if (!invoke || typeof url !== 'string' || url.trim() === '') return;
          invoke('rpc_invoke', {
            payload: {
              namespace: 'system',
              method: 'openExternal',
              params: { url }
            }
          }).catch(() => {});
        };

        const originalOpen = window.open;
        window.open = function(url, target, features) {
          if (typeof url === 'string' && /^https?:\/\//i.test(url)) {
            openExternal(url);
            return null;
          }
          if (typeof originalOpen === 'function') {
            return originalOpen.call(window, url, target, features);
          }
          return null;
        };

        document.addEventListener('click', (event) => {
          const el = event.target instanceof Element ? event.target.closest('a[target=\"_blank\"]') : null;
          if (!el) return;
          const href = el.getAttribute('href');
          if (!href || !/^https?:\/\//i.test(href)) return;
          event.preventDefault();
          openExternal(href);
        }, true);
      })();
    "#
}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_keepawake::init())
        .on_page_load(|webview, _payload| {
            let _ = webview.eval(build_external_link_patch_script());
        })
        .on_window_event(|window, event| {
            #[cfg(target_os = "macos")]
            {
                use tauri::WindowEvent;

                if window.label() == "main" {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        let app_state = window.app_handle().state::<AppState>();
                        if !app_state.is_quitting.load(Ordering::Relaxed) {
                            // 对齐 Electron：macOS 关闭窗口时仅隐藏，不退出进程。
                            api.prevent_close();
                            let _ = window.hide();
                        }
                    }
                }
            }
        })
        .setup(|app| {
            let app_handle = app.handle();

            let startup_flags = Arc::new(RwLock::new(parse_startup_flags()));
            let is_quitting = Arc::new(AtomicBool::new(false));
            let last_runtime_event = Arc::new(StdMutex::new(None));

            // Initialize AppState module
            let app_state_service =
                Arc::new(mods::app_state::AppStateService::new(app_handle.clone()));

            // Initialize Auth module
            let auth_service = mods::auth::AuthService::new(app_handle.clone());
            let auth = Arc::new(RwLock::new(auth_service));

            // Initialize Data module
            let data_service = mods::data::DataService::new(app_handle.clone());
            let data = Arc::new(RwLock::new(data_service));

            // Initialize Config module
            let config_repository = mods::config::ConfigRepository::new(app_handle.clone());
            let config_service = mods::config::ConfigService::new(config_repository);
            let config = Arc::new(RwLock::new(config_service));

            // 对齐 Electron: fullscreen = config.fullscreen || startupFlag.fullscreen。
            let startup_from_config = {
                let service = config.blocking_read();
                let keys = vec![
                    "fullscreen".to_string(),
                    "background_keepalive".to_string(),
                    "use_vulkan".to_string(),
                ];
                service
                    .get_by_keys(&keys)
                    .unwrap_or_else(|_| serde_json::json!({}))
            };
            {
                let mut flags = startup_flags.blocking_write();
                let cfg_fullscreen = startup_from_config
                    .get("fullscreen")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                flags.fullscreen = flags.fullscreen || cfg_fullscreen;
            }

            // Initialize Streaming module
            let streaming = Arc::new(mods::streaming::StreamingService::new(app_handle.clone()));

            // Initialize Engine module
            let engine = Arc::new(Mutex::new(xbxengine_app::XbxEngineApp::with_runtime_hosts(
                Box::new(NoopXbxEngineAppHostBridge),
                Box::new(xbxengine::OhMyGamepadXbxEngineInputBackend::new()),
                Box::new(PrintlnHostBridge {
                    app_handle: app_handle.clone(),
                    state: Default::default(),
                    last_runtime_event: last_runtime_event.clone(),
                }),
            )));
            let xbxengine = Arc::new(mods::xbxengine::XbxEngineService::new(
                engine.clone(),
                last_runtime_event.clone(),
            ));

            // Initialize Gamepad module
            let gamepad_host = ohmygamepad_host::GamepadRuntimeHost::shared()
                .expect("failed to init ohmygamepad host");
            let gamepad = Arc::new(mods::gamepad::GamepadService::new(
                app_handle.clone(),
                gamepad_host.clone(),
            ));

            app.manage(AppState {
                engine: engine.clone(),
                app_state: app_state_service.clone(),
                auth: auth.clone(),
                config: config.clone(),
                data: data.clone(),
                streaming: streaming.clone(),
                xbxengine: xbxengine.clone(),
                gamepad: gamepad.clone(),
                startup_flags: startup_flags.clone(),
                is_quitting,
                last_runtime_event,
            });

            // 首次启动按参数应用 fullscreen。
            if let Some(main_window) = app_handle.get_webview_window("main") {
                let fullscreen = startup_flags.blocking_read().fullscreen;
                let _ = main_window.set_fullscreen(fullscreen);
            }

            // 启动时执行 silent auth，对齐 Electron 会话恢复语义。
            let auth_clone = auth.clone();
            let app_handle_clone = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                let mut auth_guard = auth_clone.write().await;
                let _ = auth_guard.check_authentication().await;
                let state = auth_guard.get_state();
                drop(auth_guard);

                if state.is_authenticated {
                    let _ = event_bridge::emit_auth_session_ready(
                        &app_handle_clone,
                        &state.provider,
                        state.app_level,
                    );
                }
            });

            // Spawn background tick loop
            let engine_clone = engine.clone();
            tauri::async_runtime::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_millis(50));
                loop {
                    interval.tick().await;
                    let mut lock = engine_clone.lock().await;
                    lock.tick();
                }
            });

            // Spawn gamepad snapshot subscription
            let gamepad_host_clone = gamepad_host.clone();
            let app_handle_gamepad = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                let rx = gamepad_host_clone.subscribe_runtime_snapshot();
                while let Ok(snapshot) = rx.recv() {
                    let snapshot_value =
                        serde_json::to_value(&snapshot).unwrap_or_else(|_| serde_json::json!({}));
                    let _ = event_bridge::emit_gamepad_runtime_snapshot(
                        &app_handle_gamepad,
                        &snapshot_value,
                    );

                    let devices_value = serde_json::to_value(&snapshot.devices)
                        .unwrap_or_else(|_| serde_json::json!([]));
                    let _ = event_bridge::emit_gamepad_devices_changed(
                        &app_handle_gamepad,
                        &devices_value,
                    );

                    let route_value = serde_json::to_value(&snapshot.route_target)
                        .unwrap_or_else(|_| serde_json::json!({}));
                    let _ =
                        event_bridge::emit_gamepad_route_changed(&app_handle_gamepad, &route_value);

                    for pad in &snapshot.pads {
                        let pad_value =
                            serde_json::to_value(pad).unwrap_or_else(|_| serde_json::json!({}));
                        let _ = event_bridge::emit_gamepad_pad_snapshot(
                            &app_handle_gamepad,
                            &pad_value,
                        );
                    }
                }
            });

            // 对齐 Electron powerSaveBlocker('prevent-display-sleep')。
            let _ = app_handle.tauri_plugin_keepawake().start(
                &app_handle,
                Some(KeepAwakeConfig {
                    display: true,
                    idle: false,
                    sleep: false,
                }),
            );

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![greet, rpc::rpc_invoke])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

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

        if matches!(
            event,
            tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
        ) {
            let _ = app_handle.tauri_plugin_keepawake().stop(app_handle);
        }
    });
}
