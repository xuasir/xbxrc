use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tauri::{AppHandle, Manager};
use tauri_plugin_keepawake::TauriPluginKeepawakeExt;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};

use crate::error::{AppError, AppResult};
use crate::mods;
use crate::mods::{auth::events as auth_events, gamepad::events as gamepad_events};

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

/// 页面可见 / 窗口聚焦时拉回手柄采样并跑自愈链。
/// 使用 `AppHandle` 解析主窗口，避免 `Window` / `WebviewWindow` 类型分裂；全屏冷启动的延迟补打见 `build_services`。
pub fn hint_gamepad_shell_interactive(app: &AppHandle, reason: &str) {
    use ohmygamepad_protocol::OhMyGamepadInputPolicyDto;

    let Some(window) = app.get_webview_window("main") else {
        log::warn!(
            "hint_gamepad_shell_interactive skipped reason={} (main window missing)",
            reason
        );
        return;
    };
    let Some(app_state) = window.try_state::<AppState>() else {
        log::warn!(
            "hint_gamepad_shell_interactive skipped reason={} (AppState not ready)",
            reason
        );
        return;
    };
    app_state.runtime_trace.record_event(
        "gamepad-shell",
        "shellInteractiveHint",
        None,
        serde_json::json!({
            "reason": reason,
            "windowLabel": window.label(),
        }),
    );
    let policy = app_state
        .gamepad
        .get_runtime_snapshot()
        .map(|snapshot| snapshot.input_policy)
        .unwrap_or(OhMyGamepadInputPolicyDto::Shared);
    if let Err(error) = app_state.gamepad.resume_shell_sampling(policy) {
        log::warn!(
            "Failed to resume shell gamepad sampling reason={} policy={:?} error={}",
            reason,
            policy,
            error
        );
    }
    if let Err(error) = app_state.gamepad.try_stalled_sampling_self_heal() {
        log::warn!(
            "Failed to try stalled gamepad self-heal reason={} error={}",
            reason,
            error
        );
    }
    if let Err(error) = app_state.gamepad.try_startup_sampling_self_heal() {
        log::warn!(
            "Failed to try startup gamepad self-heal reason={} error={}",
            reason,
            error
        );
    }
}

fn trace_gamepad_runtime_snapshot(
    runtime_trace: &mods::runtime_trace::RuntimeTraceRecorderRef,
    event: &str,
    snapshot: &ohmygamepad_protocol::OhMyGamepadRuntimeSnapshotDto,
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
    runtime_trace.record_event(
        "gamepad-shell",
        event,
        None,
        serde_json::json!({
            "samplingLifecycle": snapshot.sampling_lifecycle,
            "samplingHealth": snapshot.sampling_health,
            "inputPolicy": snapshot.input_policy,
            "connectedDevices": snapshot.devices.iter().filter(|device| device.connected).count(),
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

fn should_force_gamepad_startup_rearm(
    snapshot: &ohmygamepad_protocol::OhMyGamepadRuntimeSnapshotDto,
) -> bool {
    snapshot.sampling_lifecycle == ohmygamepad_protocol::OhMyGamepadSamplingLifecycleDto::Active
        && snapshot.last_sample_progress_at_ms == 0
        && snapshot.last_backend_sample_activity_at_ms > 0
        && snapshot.devices.iter().any(|device| device.connected)
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
    match state.gamepad.activate_sampling(None) {
        Ok(snapshot) => {
            trace_gamepad_runtime_snapshot(
                &state.runtime_trace,
                "initServicesActivateSamplingCompleted",
                &snapshot,
                serde_json::json!({
                    "reason": "shell-init",
                }),
            );
        }
        Err(error) => {
            state.runtime_trace.record_event(
                "gamepad-shell",
                "initServicesActivateSamplingFailed",
                None,
                serde_json::json!({
                    "reason": "shell-init",
                    "error": error,
                }),
            );
        }
    }
    {
        let gamepad = state.gamepad.clone();
        let runtime_trace = state.runtime_trace.clone();
        tauri::async_runtime::spawn(async move {
            sleep(Duration::from_millis(900)).await;
            let snapshot = match gamepad.get_runtime_snapshot() {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    runtime_trace.record_event(
                        "gamepad-shell",
                        "startupActiveSamplingCheckFailed",
                        None,
                        serde_json::json!({
                            "error": error,
                        }),
                    );
                    return;
                }
            };
            trace_gamepad_runtime_snapshot(
                &runtime_trace,
                "startupActiveSamplingCheckObserved",
                &snapshot,
                serde_json::json!({
                    "delayMs": 900,
                    "shouldForceRearm": should_force_gamepad_startup_rearm(&snapshot),
                }),
            );
            if !should_force_gamepad_startup_rearm(&snapshot) {
                return;
            }

            runtime_trace.record_event(
                "gamepad-shell",
                "startupActiveSamplingRearmTriggered",
                None,
                serde_json::json!({
                    "delayMs": 900,
                    "inputPolicy": snapshot.input_policy,
                    "samplingLifecycle": snapshot.sampling_lifecycle,
                    "lastSampleProgressAtMs": snapshot.last_sample_progress_at_ms,
                    "lastBackendSampleActivityAtMs": snapshot.last_backend_sample_activity_at_ms,
                    "connectedDevices": snapshot.devices.iter().filter(|device| device.connected).count(),
                }),
            );

            match gamepad.resume_shell_sampling(snapshot.input_policy) {
                Ok(next_snapshot) => {
                    trace_gamepad_runtime_snapshot(
                        &runtime_trace,
                        "startupActiveSamplingRearmCompleted",
                        &next_snapshot,
                        serde_json::json!({
                            "delayMs": 900,
                            "reason": "startup-active-without-progress",
                        }),
                    );
                }
                Err(error) => {
                    runtime_trace.record_event(
                        "gamepad-shell",
                        "startupActiveSamplingRearmFailed",
                        None,
                        serde_json::json!({
                            "delayMs": 900,
                            "reason": "startup-active-without-progress",
                            "error": error,
                        }),
                    );
                }
            }
        });
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

    let runtime_trace_mode = {
        let stored = config_service
            .get_by_keys(&["runtime_trace_mode".to_string()])
            .ok()
            .and_then(|value| {
                value
                    .get("runtime_trace_mode")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string)
            })
            .unwrap_or_else(|| mods::runtime_trace::default_stored_trace_mode());
        mods::runtime_trace::effective_runtime_trace_mode(&stored)
    };

    let runtime_trace = Arc::new(
        mods::runtime_trace::RuntimeTraceRecorder::new_with_mode(&runtime_trace_mode).map_err(
            |error| AppError::Internal(format!("Failed to init runtime trace: {error}")),
        )?,
    );
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
    mods::runtime_trace::apply_xbxengine_trace_logging(&runtime_trace_mode);

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

    let stats_snapshot_interval = mods::runtime_trace::stats_snapshot_interval(&runtime_trace_mode);

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

    let state = AppState {
        app_state: app_state_service,
        auth: auth_provider,
        config: config_provider,
        data: data_service,
        streaming: streaming_service,
        runtime_trace,
        xbxengine: xbxengine_service,
        gamepad: gamepad_service,
        native_video,
        startup_flags,
        is_quitting,
    };

    app.manage(state.clone());

    if let Some(main_window) = app_handle.get_webview_window("main") {
        let fullscreen = state.startup_flags.read().await.fullscreen;
        let _ = main_window.set_fullscreen(fullscreen);
        if fullscreen {
            // Xbox 大屏首开：全屏切换后 WebView 可见 / pageLoad 与 SDL 采样链时序常偏晚，
            // 仅靠首次 activate_sampling 仍可能出现“已识别设备但逻辑样本不推进”；延迟补打一轮交互提示链。
            let app_delayed = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                sleep(Duration::from_millis(500)).await;
                hint_gamepad_shell_interactive(&app_delayed, "fullscreen-cold-start");
            });
        }
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
        use ohmygamepad_protocol::{OhMyGamepadSamplingHealthDto, OhMyGamepadSamplingLifecycleDto};

        let rx = gamepad_host.subscribe_runtime_snapshot();
        let mut prev_sampling_lifecycle = OhMyGamepadSamplingLifecycleDto::Active;
        let mut prev_signature: Option<String> = None;
        let mut last_background_warm_progress_promoted_at_ms: u64 = 0;

        while !is_quitting_gamepad.load(Ordering::Relaxed) {
            if let Ok(snapshot) = rx.recv() {
                let signature = format!(
                    "{:?}|{:?}|{:?}|{}|{}|{}|{}",
                    snapshot.sampling_lifecycle,
                    snapshot.sampling_health,
                    snapshot.input_policy,
                    snapshot
                        .devices
                        .iter()
                        .filter(|device| device.connected)
                        .count(),
                    snapshot.slots.len(),
                    snapshot.last_sample_progress_at_ms,
                    snapshot.last_backend_sample_activity_at_ms,
                );
                if prev_signature.as_deref() != Some(signature.as_str()) {
                    trace_gamepad_runtime_snapshot(
                        &runtime_trace_gamepad,
                        "runtimeSnapshotTransitionObserved",
                        &snapshot,
                        serde_json::json!({
                            "previousSignature": prev_signature.clone(),
                            "signature": signature,
                        }),
                    );
                    prev_signature = Some(signature);
                }
                let snapshot_value = serde_json::to_value(&snapshot).unwrap_or_else(|e| {
                    log::warn!("Failed to serialize gamepad runtime snapshot: {}", e);
                    serde_json::json!({})
                });
                let _ = gamepad_events::emit_runtime_snapshot(&app_handle_gamepad, &snapshot_value);

                let lifecycle = snapshot.sampling_lifecycle;
                let baseline_absorbed = prev_sampling_lifecycle
                    == OhMyGamepadSamplingLifecycleDto::BackgroundWarm
                    && lifecycle == OhMyGamepadSamplingLifecycleDto::Active;
                if baseline_absorbed {
                    trace_gamepad_runtime_snapshot(
                        &runtime_trace_gamepad,
                        "inputBaselineAbsorbed",
                        &snapshot,
                        serde_json::json!({
                            "previousLifecycle": "backgroundWarm",
                            "lifecycle": "active",
                        }),
                    );
                    let payload = serde_json::json!({
                        "previousLifecycle": "backgroundWarm",
                        "lifecycle": "active",
                    });
                    let _ =
                        gamepad_events::emit_input_baseline_absorbed(&app_handle_gamepad, &payload);
                }
                prev_sampling_lifecycle = lifecycle;

                if lifecycle == OhMyGamepadSamplingLifecycleDto::Active && !baseline_absorbed {
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

                if snapshot.sampling_lifecycle == OhMyGamepadSamplingLifecycleDto::BackgroundWarm
                    && snapshot.last_sample_progress_at_ms > 0
                    && snapshot.last_sample_progress_at_ms
                        > last_background_warm_progress_promoted_at_ms
                {
                    if let Some(window) = app_handle_gamepad.get_webview_window("main") {
                        let visible = window.is_visible().unwrap_or(false);
                        let minimized = window.is_minimized().unwrap_or(false);
                        let focused = window.is_focused().unwrap_or(false);
                        if visible && !minimized {
                            last_background_warm_progress_promoted_at_ms =
                                snapshot.last_sample_progress_at_ms;
                            runtime_trace_gamepad.record_event(
                                "gamepad-shell",
                                "backgroundWarmProgressAutoPromoteRequested",
                                None,
                                serde_json::json!({
                                    "lastSampleProgressAtMs": snapshot.last_sample_progress_at_ms,
                                    "visible": visible,
                                    "minimized": minimized,
                                    "focused": focused,
                                }),
                            );
                            let _ = gamepad_provider
                                .set_sampling_lifecycle(OhMyGamepadSamplingLifecycleDto::Active);
                        }
                    }
                }

                if snapshot.sampling_health == OhMyGamepadSamplingHealthDto::AwaitingBaseline
                    && snapshot.sampling_lifecycle == OhMyGamepadSamplingLifecycleDto::Active
                {
                    let self_healed = gamepad_provider.try_startup_sampling_self_heal();
                    runtime_trace_gamepad.record_event(
                        "gamepad-shell",
                        "awaitingBaselineSelfHealTriggered",
                        None,
                        serde_json::json!({
                            "result": match &self_healed {
                                Ok(applied) => serde_json::json!({
                                    "ok": true,
                                    "applied": applied,
                                }),
                                Err(error) => serde_json::json!({
                                    "ok": false,
                                    "error": error,
                                }),
                            },
                            "samplingLifecycle": snapshot.sampling_lifecycle,
                            "samplingHealth": snapshot.sampling_health,
                        }),
                    );
                }

                if snapshot.sampling_health == OhMyGamepadSamplingHealthDto::Stalled {
                    let self_healed = gamepad_provider.try_stalled_sampling_self_heal();
                    runtime_trace_gamepad.record_event(
                        "gamepad-shell",
                        "stalledSelfHealTriggered",
                        None,
                        serde_json::json!({
                            "result": match &self_healed {
                                Ok(applied) => serde_json::json!({
                                    "ok": true,
                                    "applied": applied,
                                }),
                                Err(error) => serde_json::json!({
                                    "ok": false,
                                    "error": error,
                                }),
                            },
                            "samplingLifecycle": snapshot.sampling_lifecycle,
                            "samplingHealth": snapshot.sampling_health,
                        }),
                    );
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
