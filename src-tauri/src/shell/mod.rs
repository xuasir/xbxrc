use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tauri::{AppHandle, Manager};
use tauri_plugin_keepawake::TauriPluginKeepawakeExt;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};

use crate::error::{AppError, AppResult};
use crate::mods;
use crate::mods::gamepad::input_gate;
use crate::mods::gamepad::service::sanitize_runtime_snapshot_for_external_consumers;
use crate::mods::{auth::events as auth_events, gamepad::events as gamepad_events};

pub mod bridge;
pub mod cli;
pub mod rpc;
pub mod state;
pub mod window;

#[cfg(target_os = "windows")]
mod win_foreground;

pub use bridge::{NoopTauriEngineWindowHost, TauriEngineEventBridge, TauriEngineWindowHost};
pub use cli::parse_startup_flags;
pub use state::{AppState, StartupFlagsState};
pub use window::build_external_link_patch_script;

#[cfg(target_os = "windows")]
static GAMEPAD_COLD_START_SDL_NUDGE_TASK_SCHEDULED: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "macos")]
pub use window::handle_macos_window_event;

/// 在跑 SDL `prime/reopen` 自愈链之前，先把主窗口拉到前台并请求焦点，
/// 以贴近用户「切到后台再回来」后输入才恢复的系统行为（部分蓝牙/虚拟手柄）。
pub fn request_main_window_focus_for_input_stack(app: &AppHandle, reason: &str) {
    let Some(window) = app.get_webview_window("main") else {
        log::warn!(
            "request_main_window_focus_for_input_stack skipped reason={} (main window missing)",
            reason
        );
        return;
    };
    let _ = window.show();
    #[cfg(target_os = "windows")]
    let used_win32_foreground = win_foreground::try_force_webview_foreground_win32(&window, reason);
    #[cfg(not(target_os = "windows"))]
    let used_win32_foreground = false;
    if let Err(error) = window.set_focus() {
        log::warn!(
            "request_main_window_focus_for_input_stack set_focus failed reason={} error={}",
            reason,
            error
        );
    }
    if let Some(app_state) = window.try_state::<AppState>() {
        app_state.runtime_trace.record_event(
            "gamepad-shell",
            "mainWindowFocusRequestedForInputRecovery",
            None,
            serde_json::json!({
                "reason": reason,
                "windowLabel": window.label(),
                "usedWin32Foreground": used_win32_foreground,
            }),
        );
    }
}

/// 页面可见 / 窗口聚焦时拉回手柄采样并跑轻恢复。
/// 使用 `AppHandle` 解析主窗口，避免 `Window` / `WebviewWindow` 类型分裂；全屏冷启动的延迟补打见 `build_services`。
///
/// UI/串流输入归属由前端消费层门控；RTC 是否转发样本由 `set_stream_pad_forwarding` 控制（见 RFC gamepad lifecycle simplification）。
pub fn hint_gamepad_shell_interactive(app: &AppHandle, reason: &str) {
    input_gate::sync_gamepad_input_gate(app);
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
            "streamPadForwarding": app_state.gamepad.stream_pad_forwarding(),
        }),
    );
    if let Err(error) = app_state.gamepad.resume_shell_sampling() {
        log::warn!(
            "Failed to resume shell gamepad sampling reason={} error={}",
            reason,
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
}

/// Windows：主窗口首帧 `pageLoad` 且可见后，调度 **单条** 异步任务。
/// 首轮执行一次 WinAPI/Tauri 聚焦与一次启动恢复；后续两轮仅保留观测补打，不再重复前台抢占或 startup reopen。
///
/// 由配置 `gamepad_cold_start_sdl_binding_nudge` 控制，默认开启；全进程仅 **投递一次** 该异步任务。
#[cfg(target_os = "windows")]
pub fn schedule_gamepad_cold_start_sdl_binding_nudge(app: &AppHandle) {
    if GAMEPAD_COLD_START_SDL_NUDGE_TASK_SCHEDULED.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // 相对上一阶段的等待（毫秒），累计约 150ms / 3s / 8s 触发三次。
        const PHASE_DELAYS_MS: [(u32, u64); 3] = [(0, 150), (1, 2850), (2, 5000)];
        let mut config_checked = false;

        for (phase_index, delay_ms) in PHASE_DELAYS_MS {
            sleep(Duration::from_millis(delay_ms)).await;

            let Some(state) = app.try_state::<AppState>() else {
                log::warn!("gamepad_cold_start_sdl_binding_nudge skipped (AppState not ready)");
                return;
            };
            if !config_checked {
                let enabled = match state
                    .config
                    .get_by_keys(&[String::from("gamepad_cold_start_sdl_binding_nudge")])
                {
                    Ok(value) => value
                        .get("gamepad_cold_start_sdl_binding_nudge")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true),
                    Err(_) => true,
                };
                if !enabled {
                    log::info!("gamepad_cold_start_sdl_binding_nudge disabled by config");
                    return;
                }
                config_checked = true;
            }

            state.runtime_trace.record_event(
                "gamepad-shell",
                "gamepadColdStartSdlBindingNudgeRun",
                None,
                serde_json::json!({
                    "phase": "begin",
                    "phaseIndex": phase_index,
                }),
            );

            if phase_index == 0 {
                request_main_window_focus_for_input_stack(&app, "cold-start-sdl-binding-nudge");
                if let Err(error) = state.gamepad.set_sampling_lifecycle(
                    ohmygamepad_protocol::OhMyGamepadSamplingLifecycleDto::BackgroundWarm,
                ) {
                    log::warn!(
                        "gamepad_cold_start_sdl_binding_nudge BackgroundWarm failed error={}",
                        error
                    );
                }

                sleep(Duration::from_millis(80)).await;

                let Some(state) = app.try_state::<AppState>() else {
                    log::warn!("gamepad_cold_start_sdl_binding_nudge mid-run lost AppState");
                    return;
                };

                if let Err(error) = state.gamepad.set_sampling_lifecycle(
                    ohmygamepad_protocol::OhMyGamepadSamplingLifecycleDto::Active,
                ) {
                    log::warn!(
                        "gamepad_cold_start_sdl_binding_nudge Active failed error={}",
                        error
                    );
                }

                if let Err(error) = state.gamepad.resume_shell_sampling() {
                    log::warn!(
                        "gamepad_cold_start_sdl_binding_nudge resume_shell_sampling failed error={}",
                        error
                    );
                }
            }

            if let Ok(snapshot) = state.gamepad.get_runtime_snapshot() {
                trace_gamepad_runtime_snapshot(
                    &state.runtime_trace,
                    "gamepadColdStartSdlBindingNudgeCompleted",
                    &snapshot,
                    state.gamepad.stream_pad_forwarding(),
                    serde_json::json!({
                        "phase": "done",
                        "phaseIndex": phase_index,
                    }),
                );
            }
        }
    });
}

fn trace_gamepad_runtime_snapshot(
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
    runtime_trace.record_event(
        "gamepad-shell",
        event,
        None,
        serde_json::json!({
            "samplingLifecycle": snapshot.sampling_lifecycle,
            "samplingHealth": snapshot.sampling_health,
            "streamPadForwarding": stream_pad_forwarding,
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

fn should_auto_promote_background_warm(
    snapshot: &ohmygamepad_protocol::OhMyGamepadRuntimeSnapshotDto,
    visible: bool,
    minimized: bool,
    focused: bool,
    last_background_warm_progress_promoted_at_ms: u64,
) -> bool {
    snapshot.sampling_lifecycle
        == ohmygamepad_protocol::OhMyGamepadSamplingLifecycleDto::BackgroundWarm
        && snapshot.last_sample_progress_at_ms > 0
        && snapshot.last_sample_progress_at_ms > last_background_warm_progress_promoted_at_ms
        && visible
        && !minimized
        && focused
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
    input_gate::sync_gamepad_input_gate(app.handle());
    match state.gamepad.activate_sampling() {
        Ok(snapshot) => {
            trace_gamepad_runtime_snapshot(
                &state.runtime_trace,
                "initServicesActivateSamplingCompleted",
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
                gamepad.stream_pad_forwarding(),
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
                    "streamPadForwarding": gamepad.stream_pad_forwarding(),
                    "samplingLifecycle": snapshot.sampling_lifecycle,
                    "lastSampleProgressAtMs": snapshot.last_sample_progress_at_ms,
                    "lastBackendSampleActivityAtMs": snapshot.last_backend_sample_activity_at_ms,
                    "connectedDevices": snapshot.devices.iter().filter(|device| device.connected).count(),
                }),
            );

            #[cfg(target_os = "windows")]
            {
                runtime_trace.record_event(
                    "gamepad-shell",
                    "startupActiveSamplingRearmSkipped",
                    None,
                    serde_json::json!({
                        "delayMs": 900,
                        "reason": "windows-cold-start-nudge-owns-single-reopen",
                    }),
                );
                return;
            }

            #[cfg(not(target_os = "windows"))]
            match gamepad.resume_shell_sampling() {
                Ok(next_snapshot) => {
                    trace_gamepad_runtime_snapshot(
                        &runtime_trace,
                        "startupActiveSamplingRearmCompleted",
                        &next_snapshot,
                        gamepad.stream_pad_forwarding(),
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
            // 仅靠首次 activate_sampling 仍可能出现“已识别设备但逻辑样本不推进”；延迟补打交互提示链。
            // 部分环境（尤其虚拟手柄/手表）需多轮 resume+prime/reopen 后才有稳定的逻辑槽位增量；
            // 否则用户只能依赖失焦再回前台触发同一套链。
            for (delay_ms, reason) in [
                (500u64, "fullscreen-cold-start"),
                (2000u64, "fullscreen-cold-start-delay-2s"),
                (4000u64, "fullscreen-cold-start-delay-4s"),
            ] {
                let app_delayed = app_handle.clone();
                tauri::async_runtime::spawn(async move {
                    sleep(Duration::from_millis(delay_ms)).await;
                    hint_gamepad_shell_interactive(&app_delayed, reason);
                });
            }
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
        use ohmygamepad_protocol::{
            OhMyGamepadInputGateModeDto, OhMyGamepadSamplingHealthDto,
            OhMyGamepadSamplingLifecycleDto,
        };

        let rx = gamepad_host.subscribe_runtime_snapshot();
        let mut prev_input_gate = OhMyGamepadInputGateModeDto::default();
        let mut prev_signature: Option<String> = None;
        let mut last_background_warm_progress_promoted_at_ms: u64 = 0;

        while !is_quitting_gamepad.load(Ordering::Relaxed) {
            if let Ok(snapshot) = rx.recv() {
                let signature = format!(
                    "{:?}|{:?}|{:?}|{}|{}|{}|{}|{}",
                    snapshot.sampling_lifecycle,
                    snapshot.sampling_health,
                    snapshot.input_gate,
                    gamepad_provider.stream_pad_forwarding(),
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

                let previous_gate = prev_input_gate;
                if snapshot.input_gate != prev_input_gate {
                    let gate_payload = serde_json::json!({
                        "previousGate": previous_gate,
                        "inputGate": snapshot.input_gate,
                        "reason": snapshot.input_gate_reason,
                    });
                    let _ =
                        gamepad_events::emit_input_gate_changed(&app_handle_gamepad, &gate_payload);
                    prev_input_gate = snapshot.input_gate;
                }

                if snapshot.input_gate.allows_business_input() {
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
                    let visible = window.is_visible().unwrap_or(false);
                    let minimized = window.is_minimized().unwrap_or(false);
                    let focused = window.is_focused().unwrap_or(false);
                    if should_auto_promote_background_warm(
                        &snapshot,
                        visible,
                        minimized,
                        focused,
                        last_background_warm_progress_promoted_at_ms,
                    ) {
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
                        let promote_result = gamepad_provider
                            .set_sampling_lifecycle(OhMyGamepadSamplingLifecycleDto::Active);
                        if promote_result.is_ok() {
                            // `WindowEvent::Focused(true)` 在部分 Windows/WebView2 回焦路径上可能缺失；
                            // 此处已与 `should_auto_promote_background_warm` 共用同一 `is_focused()` 判定，
                            // 升 Active 成功后必须把门控焦点位对齐，否则 `input_gate` 永久 Closed。
                            input_gate::record_shell_main_window_focused_from_os_event(true);
                            input_gate::sync_gamepad_input_gate(&app_handle_gamepad);
                        }
                    }
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

#[cfg(test)]
mod tests {
    use super::{should_auto_promote_background_warm, should_force_gamepad_startup_rearm};
    use ohmygamepad_protocol::{OhMyGamepadRuntimeSnapshotDto, OhMyGamepadSamplingLifecycleDto};

    #[test]
    fn startup_rearm_stays_independent_from_focus_state() {
        let snapshot = OhMyGamepadRuntimeSnapshotDto {
            sampling_lifecycle: OhMyGamepadSamplingLifecycleDto::Active,
            last_sample_progress_at_ms: 0,
            last_backend_sample_activity_at_ms: 42,
            devices: vec![ohmygamepad_protocol::OhMyGamepadDeviceDto {
                connected: true,
                ..Default::default()
            }],
            ..Default::default()
        };

        assert!(should_force_gamepad_startup_rearm(&snapshot));
    }

    #[test]
    fn background_warm_auto_promote_requires_window_focus() {
        let snapshot = OhMyGamepadRuntimeSnapshotDto {
            sampling_lifecycle: OhMyGamepadSamplingLifecycleDto::BackgroundWarm,
            last_sample_progress_at_ms: 1284,
            ..Default::default()
        };

        assert!(!should_auto_promote_background_warm(
            &snapshot, true, false, false, 0,
        ));
        assert!(should_auto_promote_background_warm(
            &snapshot, true, false, true, 0,
        ));
    }
}
