use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tauri::{AppHandle, Manager};
use tauri_plugin_keepawake::TauriPluginKeepawakeExt;
use tokio::sync::RwLock;

use crate::error::{AppError, AppResult};
use crate::mods;
use crate::mods::auth::events as auth_events;

pub mod bridge;
pub mod cli;
pub mod rpc;
pub mod state;
pub mod window;

pub use bridge::{NoopTauriEngineWindowHost, TauriEngineEventBridge, TauriEngineWindowHost};
pub use cli::parse_startup_flags;
pub use state::{AppState, StartupFlagsState};
pub use window::build_external_link_patch_script;

#[cfg(target_os = "macos")]
pub use window::handle_macos_window_event;

pub async fn init_services(app: &mut tauri::App) -> AppResult<()> {
    log::info!("Starting application initialization...");

    let startup_flags = load_startup_flags(app).await?;

    let state = build_services(app, startup_flags.clone()).await?;
    state.runtime_trace.record_event(
        "gamepad-shell",
        "initServicesBuilt",
        None,
        serde_json::json!({
            "fullscreen": state.startup_flags.read().await.fullscreen,
        }),
    );

    bind_background_tasks(app.handle(), &state).await?;
    log::info!("Application initialization completed.");

    Ok(())
}

async fn load_startup_flags(app: &tauri::App) -> AppResult<Arc<RwLock<StartupFlagsState>>> {
    let mut flags = parse_startup_flags();
    let app_handle = app.handle();

    let config_service = mods::config::ConfigService::new(app_handle.clone());

    let keys = vec![
        "fullscreen".to_string(),
        "background_keepalive".to_string(),
        "use_vulkan".to_string(),
    ];

    match config_service.get_by_keys(&keys) {
        Ok(config_values) => {
            let cfg_fullscreen = config_values
                .get("fullscreen")
                .and_then(|value| value.as_bool())
                .unwrap_or_else(|| {
                    log::warn!("Config 'fullscreen' missing or invalid, using default: false");
                    false
                });
            flags.fullscreen = flags.fullscreen || cfg_fullscreen;
        }
        Err(error) => {
            log::warn!("Failed to load startup config values: {}", error);
        }
    }

    Ok(Arc::new(RwLock::new(flags)))
}

async fn build_services(
    app: &mut tauri::App,
    startup_flags: Arc<RwLock<StartupFlagsState>>,
) -> AppResult<AppState> {
    let app_handle = app.handle();
    let is_quitting = Arc::new(AtomicBool::new(false));
    let last_runtime_event = Arc::new(StdMutex::new(None));

    let config_service = Arc::new(mods::config::ConfigService::new(app_handle.clone()));
    let config_provider: mods::config::ConfigProviderRef = config_service.clone();

    let (runtime_trace_mode, runtime_trace_dimensions) = {
        let stored = config_service
            .get_by_keys(&[
                "runtime_trace_mode".to_string(),
                "runtime_trace_dimensions".to_string(),
            ])
            .ok()
            .unwrap_or_default();
        let stored_mode = stored
            .get("runtime_trace_mode")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string)
            .unwrap_or_else(|| mods::runtime_trace::default_stored_trace_mode());
        let dimensions = if cfg!(debug_assertions) {
            std::env::var("XBX_TRACE_DIMENSIONS")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .or_else(|| {
                    stored
                        .get("runtime_trace_dimensions")
                        .and_then(|value| value.as_str())
                        .map(std::string::ToString::to_string)
                })
        } else {
            None
        };
        (
            mods::runtime_trace::effective_runtime_trace_mode(&stored_mode),
            dimensions,
        )
    };

    // trace 属于诊断设施，不能因为落盘失败阻断启动：
    // 先绑定应用日志目录，再让初始化失败降级为 off。
    let trace_root = app_handle
        .path()
        .app_log_dir()
        .map(|dir| dir.join("runtime-logs"))
        .unwrap_or_else(|_| std::env::temp_dir().join("xbxrc").join("runtime-logs"));
    mods::runtime_trace::set_trace_root_dir(trace_root);

    let runtime_trace = {
        let build_recorder = |mode: &str, dimensions: Option<&str>| {
            mods::runtime_trace::RuntimeTraceRecorder::new_with_config(mode, dimensions)
        };
        match build_recorder(&runtime_trace_mode, runtime_trace_dimensions.as_deref()) {
            Ok(recorder) => Arc::new(recorder),
            Err(error) => {
                log::warn!(
                    "Runtime trace init failed (mode={runtime_trace_mode}): {error}; fallback to off"
                );
                Arc::new(build_recorder("off", None).map_err(|error| {
                    AppError::Internal(format!("Failed to init runtime trace: {error}"))
                })?)
            }
        }
    };
    let native_video = Arc::new(StdMutex::new(mods::native_video::NativeVideoRegistry::new(
        app_handle.clone(),
        Some(runtime_trace.clone()),
    )));
    mods::native_video::set_runtime_trace_recorder(runtime_trace.clone());
    xbxengine::set_log_sink({
        let runtime_trace = runtime_trace.clone();
        Arc::new(move |record| {
            runtime_trace.record_log(
                "xbxengine",
                "runtimeLog",
                None,
                serde_json::json!({
                    "level": record.level.as_str(),
                    "message": record.message,
                    "tsMs": record.ts_ms,
                }),
            );
        })
    });
    ohmygamepad_sdl3::set_runtime_trace_sink(Some({
        let runtime_trace = runtime_trace.clone();
        Arc::new(move |event, payload| {
            runtime_trace.record_event("gamepad-source", event, None, payload);
        })
    }));
    mods::runtime_trace::apply_xbxengine_trace_logging(
        runtime_trace.trace_profile(),
        runtime_trace.trace_dimensions(),
    );

    if let Some(path) = runtime_trace.path() {
        log::info!(
            "Runtime trace recorder initialized at {} (mode={})",
            path.display(),
            runtime_trace.trace_mode()
        );
    } else {
        log::info!(
            "Runtime trace file logging disabled (mode={})",
            runtime_trace.trace_mode()
        );
    }

    if runtime_trace.disk_enabled() {
        runtime_trace.record_state(
            "xbxengine",
            "runtimeBuildInfo",
            None,
            serde_json::json!({
                "buildFingerprint": crate::mods::xbxengine::build_info::current_build_fingerprint(),
            }),
        );
    }

    let stats_snapshot_interval =
        mods::runtime_trace::stats_snapshot_interval(runtime_trace.trace_profile());

    let auth_service = Arc::new(mods::auth::AuthService::new(
        app_handle.clone(),
        config_provider.clone(),
    ));
    let auth_provider: mods::auth::AuthProviderRef = auth_service.clone();

    let data_service = Arc::new(mods::data::DataService::new(
        app_handle.clone(),
        auth_provider.clone(),
        config_provider.clone(),
        runtime_trace.clone(),
    ));
    let xbxengine_service = Arc::new(mods::xbxengine::XbxEngineService::new(
        app_handle.clone(),
        last_runtime_event.clone(),
        native_video.clone(),
        runtime_trace.clone(),
        stats_snapshot_interval,
    ));
    let streaming_service = Arc::new(mods::streaming::StreamingService::new(
        app_handle.clone(),
        auth_provider.clone(),
        config_provider.clone(),
        data_service.clone(),
        xbxengine_service.clone(),
        runtime_trace.clone(),
    ));

    let gamepad_service = Arc::new(mods::gamepad::GamepadService::new(app_handle.clone()));

    let app_state_service: mods::app_state::AppStateProviderRef =
        Arc::new(mods::app_state::AppStateService::new(
            app_handle.clone(),
            auth_provider.clone(),
            startup_flags.clone(),
        ));
    let updater_service = Arc::new(mods::updater::UpdaterService::new(app_handle.clone()));

    let state = AppState {
        app_state: app_state_service,
        auth: auth_provider,
        config: config_provider,
        data: data_service,
        streaming: streaming_service,
        runtime_trace,
        xbxengine: xbxengine_service,
        gamepad: gamepad_service,
        updater: updater_service,
        native_video,
        startup_flags,
        is_quitting,
    };

    app.manage(state.clone());

    if let Some(main_window) = app_handle.get_webview_window("main") {
        let fullscreen = state.startup_flags.read().await.fullscreen;
        let _ = main_window.set_fullscreen(fullscreen);
    }
    mods::native_video::configure_main_window_video_host(&app_handle);

    Ok(state)
}

async fn bind_background_tasks(app_handle: &AppHandle, state: &AppState) -> AppResult<()> {
    let auth = state.auth.clone();
    let app_handle_clone = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        let initial_state = auth.get_state();
        if !initial_state.is_authenticated && !initial_state.is_authenticating {
            let _ = auth.check_authentication().await;
        }
        let auth_state = auth.get_state();
        if auth_state.is_authenticated {
            let _ = auth_events::emit_session_ready(
                &app_handle_clone,
                &auth_state.provider,
                auth_state.app_level,
            );
        }
    });

    state.xbxengine.bind_tasks(state.is_quitting.clone());

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

pub async fn terminate(app_handle: &AppHandle) {
    log::info!("Starting application termination...");

    let Some(state) = app_handle.try_state::<AppState>() else {
        let _ = app_handle.tauri_plugin_keepawake().stop(app_handle);
        log::warn!("AppState is not available during terminate, skipped runtime shutdown.");
        return;
    };

    if state
        .is_quitting
        .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
        .is_err()
    {
        log::info!("Termination already in progress, skipping duplicated request.");
        return;
    }

    state.runtime_trace.record_event(
        "app",
        "terminateRequested",
        None,
        serde_json::json!({
            "source": "shell",
        }),
    );

    state.gamepad.shutdown();
    state.xbxengine.shutdown().await;
    state.streaming.shutdown().await;

    let _ = app_handle.tauri_plugin_keepawake().stop(app_handle);

    state.runtime_trace.record_event(
        "app",
        "terminateCompleted",
        None,
        serde_json::json!({
            "source": "shell",
        }),
    );

    log::info!("Application termination completed.");
}
