use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tauri_plugin_keepawake::TauriPluginKeepawakeExt;
use tokio::sync::{Mutex, RwLock};
use xbxengine_app::NoopXbxEngineAppHostBridge;

use crate::error::{AppError, AppResult};
use crate::mods;
use crate::mods::config::repository::ConfigRepository;
use crate::mods::{auth::events as auth_events, gamepad::events as gamepad_events};

pub mod bridge;
pub mod cli;
pub mod rpc;
pub mod state;
pub mod window;

pub use bridge::PrintlnHostBridge;
pub use cli::parse_startup_flags;
pub use state::{AppState, StartupFlagsState};
pub use window::build_external_link_patch_script;

#[cfg(target_os = "macos")]
pub use window::handle_macos_window_event;

/// 初始化所有服务和状态
pub async fn init_services(app: &mut tauri::App) -> AppResult<()> {
    log::info!("Starting application initialization...");

    // 1. load_startup_flags
    let startup_flags = load_startup_flags(app).await?;

    // 2. build_services
    let state = build_services(app, startup_flags.clone()).await?;

    // 3. bind_background_tasks
    bind_background_tasks(app.handle(), &state).await?;
    log::info!("Application initialization completed.");

    Ok(())
}

/// 1. 加载启动标志与配置
async fn load_startup_flags(app: &tauri::App) -> AppResult<Arc<RwLock<StartupFlagsState>>> {
    let mut flags = parse_startup_flags();
    let app_handle = app.handle();

    // 从配置中读取持久化设置
    let config_repository = ConfigRepository::new(app_handle.clone());
    let config_service = mods::config::ConfigService::new(config_repository);

    let keys = vec![
        "fullscreen".to_string(),
        "background_keepalive".to_string(),
        "use_vulkan".to_string(),
    ];

    if let Ok(config_values) = config_service.get_by_keys(&keys) {
        let cfg_fullscreen = config_values
            .get("fullscreen")
            .and_then(|value| value.as_bool())
            .unwrap_or_else(|| {
                log::warn!("Config 'fullscreen' missing or invalid, using default: false");
                false
            });
        flags.fullscreen = flags.fullscreen || cfg_fullscreen;
    }

    Ok(Arc::new(RwLock::new(flags)))
}

/// 2. 构建服务并同步依赖
async fn build_services(
    app: &mut tauri::App,
    startup_flags: Arc<RwLock<StartupFlagsState>>,
) -> AppResult<AppState> {
    let app_handle = app.handle();
    let is_quitting = Arc::new(AtomicBool::new(false));
    let last_runtime_event = Arc::new(StdMutex::new(None));

    // a. 基础配置与认证
    let config_repository = ConfigRepository::new(app_handle.clone());
    let config_service = Arc::new(mods::config::ConfigService::new(config_repository));
    let config_provider: mods::config::ConfigProviderRef = config_service.clone();

    let auth_service = Arc::new(mods::auth::AuthService::new(
        app_handle.clone(),
        config_provider.clone(),
    ));
    let auth_provider: mods::auth::AuthProviderRef = auth_service.clone();

    // b. 数据与流控
    let data_service = Arc::new(mods::data::DataService::new(
        app_handle.clone(),
        auth_provider.clone(),
        config_provider.clone(),
    ));
    let streaming_service = Arc::new(mods::streaming::StreamingService::new(
        auth_provider.clone(),
        config_provider.clone(),
    ));

    // c. 引擎与输入
    let engine = Arc::new(Mutex::new(xbxengine_app::XbxEngineApp::with_runtime_hosts(
        Box::new(NoopXbxEngineAppHostBridge),
        Box::new(xbxengine::OhMyGamepadXbxEngineInputBackend::new()),
        Box::new(PrintlnHostBridge {
            app_handle: app_handle.clone(),
            state: Default::default(),
            last_runtime_event: last_runtime_event.clone(),
        }),
    )));
    let xbxengine_service = Arc::new(mods::xbxengine::XbxEngineService::new(
        engine.clone(),
        last_runtime_event.clone(),
    ));

    let gamepad_host = ohmygamepad_host::GamepadRuntimeHost::shared()
        .map_err(|e| AppError::Internal(format!("Failed to init ohmygamepad host: {}", e)))?;
    let gamepad_service = Arc::new(mods::gamepad::GamepadService::new(
        app_handle.clone(),
        gamepad_host.clone(),
    ));

    // d. 全局状态编排器
    let app_state_service: mods::app_state::AppStateProviderRef =
        Arc::new(mods::app_state::AppStateService::new(
            app_handle.clone(),
            auth_provider.clone(),
            startup_flags.clone(),
        ));

    let state = AppState {
        app_state: app_state_service,
        auth: auth_provider,
        config: config_provider,
        data: data_service,
        streaming: streaming_service,
        xbxengine: xbxengine_service,
        gamepad: gamepad_service,
        startup_flags,
        is_quitting,
    };

    // 注入 Tauri 状态
    app.manage(state.clone());

    // 应用窗口初始状态
    if let Some(main_window) = app_handle.get_webview_window("main") {
        let fullscreen = state.startup_flags.read().await.fullscreen;
        let _ = main_window.set_fullscreen(fullscreen);
    }

    Ok(state)
}

/// 3. 绑定后台任务与订阅
async fn bind_background_tasks(app_handle: &AppHandle, state: &AppState) -> AppResult<()> {
    // a. 异步认证恢复
    let auth = state.auth.clone();
    let app_handle_clone = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        let _ = auth.check_authentication().await;
        let auth_state = auth.get_state();
        if auth_state.is_authenticated {
            let _ = auth_events::emit_session_ready(
                &app_handle_clone,
                &auth_state.provider,
                auth_state.app_level,
            );
        }
    });

    // b. 引擎 Tick 循环 (具备退出检查)
    state.xbxengine.bind_tasks(state.is_quitting.clone());

    // c. Gamepad 事件订阅 (具备退出检查)
    let gamepad_host = ohmygamepad_host::GamepadRuntimeHost::shared()
        .map_err(|e| AppError::Internal(format!("Failed to get gamepad host: {}", e)))?;
    let app_handle_gamepad = app_handle.clone();
    let is_quitting_gamepad = state.is_quitting.clone();
    tauri::async_runtime::spawn(async move {
        let rx = gamepad_host.subscribe_runtime_snapshot();
        let mut last_high_freq_emit = std::time::Instant::now();
        let throttle_ms = 20; // 约 50Hz，对于 UI 反馈足够

        while !is_quitting_gamepad.load(Ordering::Relaxed) {
            if let Ok(snapshot) = rx.recv() {
                let now = std::time::Instant::now();
                let should_emit_high_freq =
                    now.duration_since(last_high_freq_emit).as_millis() >= throttle_ms;

                if should_emit_high_freq {
                    let snapshot_value =
                        serde_json::to_value(&snapshot).unwrap_or_else(|e| {
                            log::warn!("Failed to serialize gamepad runtime snapshot: {}", e);
                            serde_json::json!({})
                        });
                    let _ =
                        gamepad_events::emit_runtime_snapshot(&app_handle_gamepad, &snapshot_value);

                    for pad in &snapshot.pads {
                        let pad_value =
                            serde_json::to_value(pad).unwrap_or_else(|e| {
                                log::warn!("Failed to serialize gamepad pad snapshot: {}", e);
                                serde_json::json!({})
                            });
                        let _ = gamepad_events::emit_pad_snapshot(&app_handle_gamepad, &pad_value);
                    }
                    last_high_freq_emit = now;
                }

                let devices_value = serde_json::to_value(&snapshot.devices)
                    .unwrap_or_else(|e| {
                        log::warn!("Failed to serialize gamepad devices: {}", e);
                        serde_json::json!([])
                    });
                let _ = gamepad_events::emit_devices_changed(&app_handle_gamepad, &devices_value);

                let route_value = serde_json::to_value(&snapshot.route_target)
                    .unwrap_or_else(|e| {
                        log::warn!("Failed to serialize gamepad route target: {}", e);
                        serde_json::json!({})
                    });
                let _ = gamepad_events::emit_route_changed(&app_handle_gamepad, &route_value);
            } else {
                break;
            }
        }
        log::info!("Gamepad subscription loop stopped.");
    });

    // d. 阻止休眠
    let _ = app_handle.tauri_plugin_keepawake().start(
        app_handle,
        Some(tauri_plugin_keepawake::KeepAwakeConfig {
            display: true,
            idle: false,
            sleep: false,
        }),
    );

    Ok(())
}

/// 退出流程收敛函数
pub async fn terminate(app_handle: &AppHandle) {
    log::info!("Starting application termination...");

    // 获取全局状态
    let state = app_handle.state::<AppState>();

    // 1. set_quitting_flag
    state.is_quitting.store(true, Ordering::Relaxed);

    // 2. shutdown_runtime_services (按依赖反序关闭)
    state.gamepad.shutdown();
    state.xbxengine.shutdown().await;
    state.streaming.shutdown().await;

    // 3. release_os_resources
    let _ = app_handle.tauri_plugin_keepawake().stop(app_handle);

    log::info!("Application termination completed.");
}
