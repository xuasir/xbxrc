use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tauri::{AppHandle, Manager};
use tauri_plugin_keepawake::TauriPluginKeepawakeExt;
use tokio::sync::RwLock;

use crate::error::{AppError, AppResult};
use crate::mods;
use crate::mods::gamepad::input_route;
use crate::mods::gamepad::service::sanitize_runtime_snapshot_for_external_consumers;
use crate::mods::{auth::events as auth_events, gamepad::events as gamepad_events};

pub mod bridge;
pub mod cli;
mod gamepad_sampling;
pub mod rpc;
pub mod state;
pub mod window;

pub use bridge::{NoopTauriEngineWindowHost, TauriEngineEventBridge, TauriEngineWindowHost};
pub use cli::parse_startup_flags;
pub use gamepad_sampling::{
    handle_gamepad_app_resumed, handle_gamepad_page_loaded, handle_gamepad_window_focus_changed,
    handle_gamepad_window_resized,
};
pub use state::{AppState, StartupFlagsState};
pub use window::build_external_link_patch_script;

#[cfg(target_os = "macos")]
pub use window::handle_macos_window_event;

pub(super) fn trace_gamepad_runtime_snapshot(
    runtime_trace: &mods::runtime_trace::RuntimeTraceRecorderRef,
    event: &str,
    snapshot: &ohmygamepad_protocol::OhMyGamepadRuntimeSnapshotDto,
    stream_pad_forwarding: bool,
    payload: serde_json::Value,
) {
    let max_sample_seq = snapshot
        .slots
        .iter()
        .map(|slot| slot.sample_seq)
        .max()
        .unwrap_or(0);
    let max_sampled_at_ms = snapshot
        .slots
        .iter()
        .map(|slot| slot.sampled_at_ms)
        .max()
        .unwrap_or(0);
    let connected_device_ids = connected_gamepad_device_ids(snapshot);
    let keyboard_fallback_connected = snapshot
        .devices
        .iter()
        .any(|device| device.connected && device.device_id == "virtual:keyboard");
    let sdl_physical_connected_devices = snapshot
        .devices
        .iter()
        .filter(|device| {
            device.connected
                && device.backend == Some(ohmygamepad_protocol::OhMyGamepadBackendKindDto::Sdl3)
                && device.device_id != "virtual:keyboard"
        })
        .count();
    runtime_trace.record_event(
        "gamepad-shell",
        event,
        None,
        serde_json::json!({
            "samplingLifecycle": snapshot.sampling_lifecycle,
            "samplingHealth": snapshot.sampling_health,
            "streamPadForwarding": stream_pad_forwarding,
            "connectedDevices": snapshot.devices.iter().filter(|device| device.connected).count(),
            "connectedDeviceIds": connected_device_ids,
            "keyboardFallbackConnected": keyboard_fallback_connected,
            "sdlPhysicalConnectedDevices": sdl_physical_connected_devices,
            "deviceSummaries": gamepad_device_trace_summaries(snapshot),
            "slotCount": snapshot.slots.len(),
            "maxSampleSeq": max_sample_seq,
            "maxSampledAtMs": max_sampled_at_ms,
            "lastSampleProgressAtMs": snapshot.last_sample_progress_at_ms,
            "lastBackendSampleActivityAtMs": snapshot.last_backend_sample_activity_at_ms,
            "samplingSelfHealCount": snapshot.sampling_self_heal_count,
            "payload": payload,
        }),
    );
}

fn connected_gamepad_device_ids(
    snapshot: &ohmygamepad_protocol::OhMyGamepadRuntimeSnapshotDto,
) -> Vec<String> {
    let mut ids = snapshot
        .devices
        .iter()
        .filter(|device| device.connected)
        .map(|device| device.device_id.clone())
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

fn gamepad_device_trace_summaries(
    snapshot: &ohmygamepad_protocol::OhMyGamepadRuntimeSnapshotDto,
) -> Vec<serde_json::Value> {
    snapshot
        .devices
        .iter()
        .filter(|device| device.connected)
        .map(|device| {
            serde_json::json!({
                "deviceId": device.device_id,
                "name": device.name,
                "backend": device.backend,
                "vendorId": device.vendor_id,
                "productId": device.product_id,
                "gamepadType": device.gamepad_type,
                "connection": device.connection,
                "isVirtualController": device.classification.is_virtual_controller,
                "isHandheldBuiltin": device.classification.is_handheld_builtin,
                "classificationReasons": device.classification.reasons,
            })
        })
        .collect()
}

fn gamepad_device_trace_signature(
    snapshot: &ohmygamepad_protocol::OhMyGamepadRuntimeSnapshotDto,
) -> String {
    connected_gamepad_device_ids(snapshot)
        .into_iter()
        .map(|device_id| {
            let backend = snapshot
                .devices
                .iter()
                .find(|device| device.device_id == device_id)
                .and_then(|device| device.backend)
                .map(|backend| format!("{backend:?}"))
                .unwrap_or_else(|| "None".to_owned());
            format!("{backend}:{device_id}")
        })
        .collect::<Vec<_>>()
        .join("|")
}

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
    input_route::sync_gamepad_input_route(app.handle());
    // 采样随 host bootstrap 常驻启动，这里只记录一次初始快照。
    match state.gamepad.get_runtime_snapshot() {
        Ok(snapshot) => {
            trace_gamepad_runtime_snapshot(
                &state.runtime_trace,
                "initServicesSamplingReady",
                &snapshot,
                state.gamepad.stream_pad_forwarding(),
                serde_json::json!({
                    "reason": "shell-init",
                }),
            );
        }
        Err(error) => {
            state.runtime_trace.record_event(
                "gamepad-shell",
                "initServicesSnapshotFailed",
                None,
                serde_json::json!({
                    "reason": "shell-init",
                    "error": error,
                }),
            );
        }
    }
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
    ohmygamepad_host::set_runtime_trace_sink(Some({
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

    let gamepad_host = ohmygamepad_host::GamepadRuntimeHost::shared()
        .map_err(|e| AppError::Internal(format!("Failed to init ohmygamepad host: {}", e)))?;
    let gamepad_service = Arc::new(mods::gamepad::GamepadService::new(
        app_handle.clone(),
        gamepad_host.clone(),
    ));

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

    let gamepad_host = ohmygamepad_host::GamepadRuntimeHost::shared()
        .map_err(|e| AppError::Internal(format!("Failed to get gamepad host: {}", e)))?;
    let gamepad_provider = state.gamepad.clone();
    let app_handle_gamepad = app_handle.clone();
    let is_quitting_gamepad = state.is_quitting.clone();
    let runtime_trace_gamepad = state.runtime_trace.clone();
    tauri::async_runtime::spawn(async move {
        use ohmygamepad_protocol::OhMyGamepadInputRouteDto;

        let rx = gamepad_host.subscribe_runtime_snapshot();
        let mut prev_input_route = OhMyGamepadInputRouteDto::default();
        let mut prev_signature: Option<String> = None;
        // 上一次同步给运行时的前台判定；变化时重下发一次 gate。
        let mut prev_synced_shell_app_active: Option<bool> = None;

        while !is_quitting_gamepad.load(Ordering::Relaxed) {
            if let Ok(snapshot) = rx.recv() {
                let signature = format!(
                    "{:?}|{:?}|{:?}|{}|{}|{}|{}|{}|{}",
                    snapshot.sampling_lifecycle,
                    snapshot.sampling_health,
                    snapshot.input_route,
                    gamepad_provider.stream_pad_forwarding(),
                    snapshot
                        .devices
                        .iter()
                        .filter(|device| device.connected)
                        .count(),
                    snapshot.slots.len(),
                    snapshot.last_sample_progress_at_ms,
                    snapshot.last_backend_sample_activity_at_ms,
                    gamepad_device_trace_signature(&snapshot),
                );
                if prev_signature.as_deref() != Some(signature.as_str()) {
                    trace_gamepad_runtime_snapshot(
                        &runtime_trace_gamepad,
                        "runtimeSnapshotTransitionObserved",
                        &snapshot,
                        gamepad_provider.stream_pad_forwarding(),
                        serde_json::json!({
                            "previousSignature": prev_signature.clone(),
                            "signature": signature,
                        }),
                    );
                    prev_signature = Some(signature);
                }
                let external_snapshot =
                    sanitize_runtime_snapshot_for_external_consumers(snapshot.clone());
                let snapshot_value = serde_json::to_value(&external_snapshot).unwrap_or_else(|e| {
                    log::warn!("Failed to serialize gamepad runtime snapshot: {}", e);
                    serde_json::json!({})
                });
                let _ = gamepad_events::emit_runtime_snapshot(&app_handle_gamepad, &snapshot_value);

                let previous_route = prev_input_route;
                if snapshot.input_route != prev_input_route {
                    let route_payload = serde_json::json!({
                        "previousRoute": previous_route,
                        "inputRoute": snapshot.input_route,
                    });
                    let _ = gamepad_events::emit_input_route_changed(
                        &app_handle_gamepad,
                        &route_payload,
                    );
                    prev_input_route = snapshot.input_route;
                }

                if snapshot.input_route.allows_business_input() {
                    for slot in &snapshot.slots {
                        let slot_value = serde_json::to_value(slot).unwrap_or_else(|e| {
                            log::warn!("Failed to serialize gamepad slot snapshot: {}", e);
                            serde_json::json!({})
                        });
                        let _ =
                            gamepad_events::emit_slot_snapshot(&app_handle_gamepad, &slot_value);
                    }
                }

                let devices_value = serde_json::to_value(&snapshot.devices).unwrap_or_else(|e| {
                    log::warn!("Failed to serialize gamepad devices: {}", e);
                    serde_json::json!([])
                });
                let _ = gamepad_events::emit_devices_changed(&app_handle_gamepad, &devices_value);

                if let Some(window) = app_handle_gamepad.get_webview_window("main") {
                    let (hints, foreground) =
                        input_route::current_shell_window_route_hints_with_state(&window);
                    let shell_app_active = hints.shell_app_active;
                    // 自愈：前台判定与上次同步不同就重下发一次，不依赖 window 事件送达。
                    if prev_synced_shell_app_active != Some(shell_app_active) {
                        prev_synced_shell_app_active = Some(shell_app_active);
                        input_route::sync_gamepad_input_route(&app_handle_gamepad);
                        runtime_trace_gamepad.record_event(
                            "gamepad-shell",
                            "inputRouteHintsResynced",
                            None,
                            serde_json::json!({
                                "shellAppActive": shell_app_active,
                                "visible": foreground.visible,
                                "minimized": foreground.minimized,
                            }),
                        );
                    }
                }
            } else {
                break;
            }
        }
        log::info!("Gamepad subscription loop stopped.");
    });

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
