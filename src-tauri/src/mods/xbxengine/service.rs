use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use std::time::Instant;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use serde_json::Value;
use tauri::AppHandle;
use xbxengine_protocol::{
    XbxEngineControlCommandDto, XbxEngineDisplayOptionsDto, XbxEngineDisplayStateDto,
    XbxEngineIceCandidatePolicyDto, XbxEngineInputEventDto, XbxEngineReconnectReasonDto,
    XbxEngineRenderProjectionDto, XbxEngineRuntimeCodecPreferenceDto,
    XbxEngineRuntimeProjectionDto, XbxEngineRuntimeRecoveryDto, XbxEngineRuntimeVideoPipelineDto,
    XbxEngineSessionDto, XbxEngineTargetTypeDto, XbxEngineTurnServerDto, XbxEngineViewportDto,
};

use super::preview_lifecycle::{PreviewRuntimeSlot, PreviewStop, StopResult};
use crate::error::{AppError, AppResult};
use crate::mods::native_video::NativeVideoRegistryRef;
use crate::mods::runtime_trace::RuntimeTraceRecorderRef;
use crate::mods::streaming::{
    StreamingRenderProjection, StreamingRuntimeProjection, StreamingTurnServerConfig,
};
use crate::mods::xbxengine::runtime_state::XbxEngineRuntimeState;
use crate::mods::xbxengine::XbxEngineProvider;

const XBXENGINE_TICK_INTERVAL_MS: u64 = 16;

fn runtime_tick_missed_behavior() -> tokio::time::MissedTickBehavior {
    tokio::time::MissedTickBehavior::Delay
}

pub struct XbxEngineService {
    runtimes: Arc<StdMutex<PreviewRuntimeSlot<PreviewRuntimeInstance>>>,
    app_handle: AppHandle,
    native_video: NativeVideoRegistryRef,
    config: crate::mods::config::ConfigProviderRef,
    is_quitting: Arc<AtomicBool>,
    stats_snapshot_interval: StdMutex<Duration>,
    last_runtime_event: Arc<StdMutex<Option<Value>>>,
    runtime_trace: RuntimeTraceRecorderRef,
}

struct PreviewRuntimeInstance {
    state: Arc<XbxEngineRuntimeState>,
    tick_task: StdMutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

impl PreviewRuntimeInstance {
    fn stop_tick(&self) {
        if let Ok(mut task) = self.tick_task.lock() {
            if let Some(task) = task.take() {
                task.abort();
            }
        }
    }
}

impl Drop for PreviewRuntimeInstance {
    fn drop(&mut self) {
        self.stop_tick();
    }
}

impl XbxEngineService {
    pub fn new(
        app_handle: AppHandle,
        last_runtime_event: Arc<StdMutex<Option<Value>>>,
        native_video: NativeVideoRegistryRef,
        runtime_trace: RuntimeTraceRecorderRef,
        stats_snapshot_interval: Duration,
        config: crate::mods::config::ConfigProviderRef,
        is_quitting: Arc<AtomicBool>,
    ) -> Self {
        Self {
            runtimes: Arc::new(StdMutex::new(PreviewRuntimeSlot::default())),
            app_handle,
            native_video,
            config,
            is_quitting,
            stats_snapshot_interval: StdMutex::new(stats_snapshot_interval),
            last_runtime_event,
            runtime_trace,
        }
    }

    fn current_runtime(&self) -> AppResult<Option<Arc<PreviewRuntimeInstance>>> {
        Ok(self
            .runtimes
            .lock()
            .map_err(|_| AppError::XbxEngine("nativeRuntimeSlotPoisoned".into()))?
            .current())
    }

    fn start_runtime(
        &self,
        command: &XbxEngineControlCommandDto,
    ) -> AppResult<(Arc<PreviewRuntimeInstance>, u64)> {
        crate::mods::config::runtime_features::require_native_feature(self.config.as_ref(), true)?;
        let interval = *self
            .stats_snapshot_interval
            .lock()
            .map_err(|_| AppError::XbxEngine("nativeStatsIntervalPoisoned".into()))?;
        let mut runtimes = self
            .runtimes
            .lock()
            .map_err(|_| AppError::XbxEngine("nativeRuntimeSlotPoisoned".into()))?;
        if self.is_quitting.load(Ordering::Acquire) {
            return Err(AppError::XbxEngine("appTerminating".into()));
        }
        let instance = runtimes
            .start(|| {
                if let Ok(mut event) = self.last_runtime_event.lock() {
                    *event = None;
                }
                PreviewRuntimeInstance {
                    state: Arc::new(XbxEngineRuntimeState::new(
                        self.app_handle.clone(),
                        self.last_runtime_event.clone(),
                        self.native_video.clone(),
                        self.runtime_trace.clone(),
                        interval,
                    )),
                    tick_task: StdMutex::new(None),
                }
            })
            .map_err(|error| AppError::XbxEngine(error.into()))?;
        crate::mods::native_video::configure_main_window_video_host(&self.app_handle);
        // 定时任务只在预览会话存在时运行；弱引用让停止后资源自然释放。
        let weak = Arc::downgrade(&instance);
        let quitting = self.is_quitting.clone();
        let task = tauri::async_runtime::spawn(async move {
            let mut interval =
                tokio::time::interval(Duration::from_millis(XBXENGINE_TICK_INTERVAL_MS));
            interval.set_missed_tick_behavior(runtime_tick_missed_behavior());
            loop {
                interval.tick().await;
                if quitting.load(Ordering::Acquire) {
                    break;
                }
                let Some(instance) = weak.upgrade() else {
                    break;
                };
                let state = instance.state.clone();
                drop(instance);
                match tauri::async_runtime::spawn_blocking(move || state.tick()).await {
                    Ok(Ok(())) => {}
                    result => log::warn!("xbxengine tick failed: {result:?}"),
                }
            }
        });
        *instance
            .tick_task
            .lock()
            .map_err(|_| AppError::XbxEngine("nativeTickTaskPoisoned".into()))? = Some(task);
        let epoch = instance.state.prepare_control(command);
        Ok((instance, epoch))
    }

    async fn stop_runtime(
        &self,
        command: XbxEngineControlCommandDto,
        expected: Option<&Arc<PreviewRuntimeInstance>>,
    ) -> AppResult<()> {
        let (stop, epoch) = {
            let mut runtimes = self
                .runtimes
                .lock()
                .map_err(|_| AppError::XbxEngine("nativeRuntimeSlotPoisoned".into()))?;
            if let Some(expected) = expected {
                if runtimes
                    .current()
                    .is_some_and(|current| !Arc::ptr_eq(&current, expected))
                {
                    return Ok(());
                }
            }
            let stop = runtimes.begin_stop();
            let epoch = match &stop {
                PreviewStop::Started { runtime, .. } => {
                    runtime.stop_tick();
                    runtime.state.prepare_control(&command)
                }
                _ => 0,
            };
            (stop, epoch)
        };
        let mut receiver = match stop {
            PreviewStop::Idle => return Ok(()),
            PreviewStop::Waiting(receiver) => receiver,
            PreviewStop::Started {
                runtime,
                completion,
            } => {
                let receiver = completion.subscribe();
                let app_handle = self.app_handle.clone();
                let runtimes = self.runtimes.clone();
                let trace = self.runtime_trace.clone();
                // 排空任务独立于 RPC 等待方；取消等待仍会完成资源回收并通知所有停止调用。
                tauri::async_runtime::spawn(async move {
                    let started_at = Instant::now();
                    record_dispatch_trace(&trace, "StopRuntime", &command, "requested", None, None);
                    let command_for_trace = command.clone();
                    let state = runtime.state.clone();
                    let result = tauri::async_runtime::spawn_blocking(move || {
                        let result = state.apply_control(command, epoch, None, None);
                        state.shutdown();
                        result
                    })
                    .await
                    .map_err(|error| {
                        AppError::XbxEngine(format!("nativeStopTaskJoinFailed: {error}"))
                    })
                    .and_then(|result| {
                        result.map_err(|error| AppError::XbxEngine(error.to_string()))
                    });
                    drop(runtime);
                    crate::mods::native_video::restore_main_window_webview_host(&app_handle);
                    record_dispatch_trace(
                        &trace,
                        "StopRuntime",
                        &command_for_trace,
                        "completed",
                        Some(started_at.elapsed().as_millis()),
                        result.as_ref().err(),
                    );
                    let shared_result = result.map_err(|error| error.to_string());
                    if let Ok(mut slot) = runtimes.lock() {
                        slot.finish_stop();
                    }
                    completion.send_replace(Some(shared_result));
                });
                receiver
            }
        };
        loop {
            let result: StopResult = receiver.borrow_and_update().clone();
            if let Some(result) = result {
                return result.map_err(AppError::XbxEngine);
            }
            receiver
                .changed()
                .await
                .map_err(|_| AppError::XbxEngine("nativeStopCompletionLost".into()))?;
        }
    }
}

#[async_trait]
impl XbxEngineProvider for XbxEngineService {
    async fn dispatch_control(&self, command_name: &str, params: Option<Value>) -> AppResult<()> {
        let expected_session_id = params
            .as_ref()
            .and_then(|params| params.get("sessionId"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let command = parse_control_command(command_name, params)?;
        let is_start = matches!(command, XbxEngineControlCommandDto::StartRuntime { .. });
        if matches!(command, XbxEngineControlCommandDto::StopRuntime { .. }) {
            return self.stop_runtime(command, None).await;
        }
        // 捕获实例与控制 epoch 在同一锁内完成，确保 Stop 撤销所有排队控制。
        let (instance, control_epoch) = if is_start {
            self.start_runtime(&command)?
        } else {
            let runtimes = self
                .runtimes
                .lock()
                .map_err(|_| AppError::XbxEngine("nativeRuntimeSlotPoisoned".into()))?;
            let instance = runtimes
                .current()
                .ok_or_else(|| AppError::XbxEngine("nativeRuntimeInactive".into()))?;
            let epoch = instance.state.prepare_control(&command);
            (instance, epoch)
        };
        record_dispatch_trace(
            &self.runtime_trace,
            command_name,
            &command,
            "requested",
            None,
            None,
        );
        let started_at = Instant::now();
        let deadline = matches!(command, XbxEngineControlCommandDto::RequestReconnect { .. })
            .then(|| started_at + Duration::from_secs(120));
        let runtime_state = instance.state.clone();
        let command_for_trace = command.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            runtime_state.apply_control(
                command,
                control_epoch,
                expected_session_id.as_deref(),
                deadline,
            )
        })
        .await
        .map_err(|error| {
            AppError::XbxEngine(format!("xbxengine control task join failed: {error}"))
        })
        .and_then(|result| result.map_err(|error| AppError::XbxEngine(error.to_string())));
        if is_start && result.is_err() {
            if let Err(error) = self
                .stop_runtime(
                    XbxEngineControlCommandDto::StopRuntime {
                        reason: Some("startFailed".into()),
                    },
                    Some(&instance),
                )
                .await
            {
                log::warn!("native runtime startup cleanup failed: {error}");
            }
        }
        record_dispatch_trace(
            &self.runtime_trace,
            command_name,
            &command_for_trace,
            "completed",
            Some(started_at.elapsed().as_millis()),
            result.as_ref().err(),
        );
        result
    }

    async fn snapshot_stats(&self) -> AppResult<Value> {
        match self.current_runtime()? {
            Some(runtime) => runtime.state.snapshot_stats(),
            None => Ok(Value::Null),
        }
    }

    fn get_last_runtime_event(&self) -> AppResult<Value> {
        let event = self
            .last_runtime_event
            .lock()
            .map_err(|_| AppError::XbxEngine("Failed to lock last runtime event".to_string()))?
            .clone();
        Ok(event.unwrap_or(Value::Null))
    }

    fn is_runtime_available(&self) -> bool {
        crate::mods::config::runtime_features::runtime_feature_capabilities().native_media
    }

    fn set_stats_snapshot_interval(&self, interval: Duration) {
        if let Ok(mut stored) = self.stats_snapshot_interval.lock() {
            *stored = interval;
        }
        if let Ok(Some(runtime)) = self.current_runtime() {
            runtime.state.set_stats_snapshot_interval(interval);
        }
    }

    async fn shutdown(&self) {
        let _ = self
            .dispatch_control(
                "StopRuntime",
                Some(serde_json::json!({
                    "reason": "appTerminate",
                })),
            )
            .await;
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn runtime_tick_does_not_skip_missed_intervals() {
        assert_eq!(
            super::runtime_tick_missed_behavior(),
            tokio::time::MissedTickBehavior::Delay
        );
    }

    #[test]
    fn parse_apply_display_state_preserves_video_format() {
        let command = super::parse_control_command(
            "ApplyDisplayState",
            Some(json!({
                "state": {
                    "video_format": "Stretch",
                    "display_options": {
                        "sharpness": 1.0,
                        "saturation": 1.1,
                        "contrast": 1.2,
                        "brightness": 1.3
                    }
                }
            })),
        )
        .expect("command should parse");

        let value = serde_json::to_value(command).expect("command should serialize");
        assert_eq!(
            value["ApplyDisplayState"]["state"]["video_format"],
            "Stretch"
        );
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartRuntimeParams {
    session: StartRuntimeSessionParams,
    viewport: AttachViewportParams,
    runtime: StreamingRuntimeProjection,
    render: StreamingRenderProjection,
    #[serde(default)]
    ice_candidate_policy: Option<IceCandidatePolicyParams>,
    audio_volume: f32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IceCandidatePolicyParams {
    enabled: bool,
    prefer_ipv6: bool,
    prefer_udp: bool,
    allow_tcp_fallback: bool,
    relay_bias: String,
    enable_teredo_derivation: bool,
    enable_family_mismatch_gate: bool,
    source: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartRuntimeSessionParams {
    session_id: String,
    target_type: String,
    turn_server: Option<StreamingTurnServerConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AttachViewportParams {
    viewport_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestReconnectParams {
    reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StopRuntimeParams {
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetAudioVolumeParams {
    value: f32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetKeyboardPointerEnabledParams {
    enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApplyDisplayStateParams {
    state: ApplyDisplayStateValue,
}

#[derive(Debug, Deserialize)]
struct ApplyDisplayStateValue {
    #[serde(default)]
    video_format: Option<String>,
    display_options: ApplyDisplayOptionsValue,
}

#[derive(Debug, Deserialize)]
struct ApplyDisplayOptionsValue {
    sharpness: f32,
    saturation: f32,
    contrast: f32,
    brightness: f32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PushKeyboardPointerInputParams {
    event: RendererInputEventDto,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum RendererInputEventDto {
    Pointer {
        at_ms: u64,
        event: String,
        pointer_type: String,
        x: f64,
        y: f64,
        delta_x: Option<f64>,
        delta_y: Option<f64>,
        button: Option<u8>,
    },
    Keyboard {
        at_ms: u64,
        event: String,
        code: String,
        key: String,
        repeat: bool,
        ctrl_key: bool,
        shift_key: bool,
        alt_key: bool,
        meta_key: bool,
    },
}

fn parse_control_command(
    command_name: &str,
    params: Option<Value>,
) -> AppResult<XbxEngineControlCommandDto> {
    match command_name {
        "StartRuntime" => {
            let params: StartRuntimeParams = parse_required_params(command_name, params)?;
            Ok(XbxEngineControlCommandDto::StartRuntime {
                session: XbxEngineSessionDto {
                    session_id: params.session.session_id,
                    target_type: match params.session.target_type.as_str() {
                        "home" => XbxEngineTargetTypeDto::Home,
                        _ => XbxEngineTargetTypeDto::Cloud,
                    },
                    turn_server: params
                        .session
                        .turn_server
                        .map(|turn| XbxEngineTurnServerDto {
                            url: turn.url,
                            username: turn.username,
                            credential: turn.credential,
                        }),
                },
                viewport: XbxEngineViewportDto {
                    viewport_id: params.viewport.viewport_id,
                },
                audio_volume: params.audio_volume,
                runtime: Some(to_runtime_projection(params.runtime)),
                render: Some(to_render_projection(params.render)),
                ice_candidate_policy: params.ice_candidate_policy.map(|policy| {
                    XbxEngineIceCandidatePolicyDto {
                        enabled: policy.enabled,
                        prefer_ipv6: policy.prefer_ipv6,
                        prefer_udp: policy.prefer_udp,
                        allow_tcp_fallback: policy.allow_tcp_fallback,
                        relay_bias: policy.relay_bias,
                        enable_teredo_derivation: policy.enable_teredo_derivation,
                        enable_family_mismatch_gate: policy.enable_family_mismatch_gate,
                        source: policy.source,
                    }
                }),
            })
        }
        "RequestReconnect" => {
            let params: RequestReconnectParams = parse_required_params(command_name, params)?;
            Ok(XbxEngineControlCommandDto::RequestReconnect {
                reason: match params.reason.as_str() {
                    "iceFailed" => XbxEngineReconnectReasonDto::IceFailed,
                    "mediaStalled" => XbxEngineReconnectReasonDto::MediaStalled,
                    _ => XbxEngineReconnectReasonDto::NetworkLost,
                },
            })
        }
        "StopRuntime" => {
            let reason = params
                .map(serde_json::from_value::<StopRuntimeParams>)
                .transpose()
                .map_err(|error| {
                    AppError::InvalidParams(format!("Invalid StopRuntime params: {error}"))
                })?
                .and_then(|value| value.reason);
            Ok(XbxEngineControlCommandDto::StopRuntime { reason })
        }
        "AttachViewport" => {
            let params: AttachViewportParams = parse_required_params(command_name, params)?;
            Ok(XbxEngineControlCommandDto::AttachViewport {
                viewport: XbxEngineViewportDto {
                    viewport_id: params.viewport_id,
                },
            })
        }
        "DetachViewport" => Ok(XbxEngineControlCommandDto::DetachViewport),
        "ApplyDisplayState" => {
            let params: ApplyDisplayStateParams = parse_required_params(command_name, params)?;
            Ok(XbxEngineControlCommandDto::ApplyDisplayState {
                state: XbxEngineDisplayStateDto {
                    video_format: params.state.video_format,
                    display_options: XbxEngineDisplayOptionsDto {
                        sharpness: params.state.display_options.sharpness,
                        saturation: params.state.display_options.saturation,
                        contrast: params.state.display_options.contrast,
                        brightness: params.state.display_options.brightness,
                    },
                },
            })
        }
        "SetKeyboardPointerEnabled" => {
            let params: SetKeyboardPointerEnabledParams =
                parse_required_params(command_name, params)?;
            Ok(XbxEngineControlCommandDto::SetKeyboardPointerEnabled {
                enabled: params.enabled,
            })
        }
        "PushKeyboardPointerInput" => {
            let params: PushKeyboardPointerInputParams =
                parse_required_params(command_name, params)?;
            Ok(XbxEngineControlCommandDto::PushKeyboardPointerInput {
                event: match params.event {
                    RendererInputEventDto::Pointer {
                        at_ms,
                        event,
                        pointer_type,
                        x,
                        y,
                        delta_x,
                        delta_y,
                        button,
                    } => XbxEngineInputEventDto::Pointer {
                        at_ms,
                        event,
                        pointer_type,
                        x,
                        y,
                        delta_x,
                        delta_y,
                        button,
                    },
                    RendererInputEventDto::Keyboard {
                        at_ms,
                        event,
                        code,
                        key,
                        repeat,
                        ctrl_key,
                        shift_key,
                        alt_key,
                        meta_key,
                    } => XbxEngineInputEventDto::Keyboard {
                        at_ms,
                        event,
                        code,
                        key,
                        repeat,
                        ctrl_key,
                        shift_key,
                        alt_key,
                        meta_key,
                    },
                },
            })
        }
        "SetAudioVolume" => {
            let params: SetAudioVolumeParams = parse_required_params(command_name, params)?;
            Ok(XbxEngineControlCommandDto::SetAudioVolume {
                value: params.value,
            })
        }
        "StartMicrophone" => Ok(XbxEngineControlCommandDto::StartMicrophone),
        "StopMicrophone" => Ok(XbxEngineControlCommandDto::StopMicrophone),
        other => Err(AppError::InvalidParams(format!(
            "Unsupported xbxengine control command: {other}"
        ))),
    }
}

fn record_dispatch_trace(
    runtime_trace: &RuntimeTraceRecorderRef,
    command_name: &str,
    command: &XbxEngineControlCommandDto,
    stage: &'static str,
    duration_ms: Option<u128>,
    error: Option<&AppError>,
) {
    let Some((event_name, session_id, viewport_id, target_type)) =
        trace_event_summary(command_name, command, stage)
    else {
        return;
    };
    runtime_trace.record_event(
        "xbxengine-host",
        event_name,
        session_id.as_deref(),
        json!({
            "command": command_name,
            "sessionId": session_id,
            "viewportId": viewport_id,
            "targetType": target_type,
            "durationMs": duration_ms,
            "ok": error.is_none(),
            "errorCode": error.map(AppError::code),
            "errorMessage": error.map(ToString::to_string),
        }),
    );
}

fn trace_event_summary(
    command_name: &str,
    command: &XbxEngineControlCommandDto,
    stage: &'static str,
) -> Option<(
    &'static str,
    Option<String>,
    Option<String>,
    Option<&'static str>,
)> {
    match (command_name, command, stage) {
        (
            "AttachViewport",
            XbxEngineControlCommandDto::AttachViewport { viewport },
            "requested",
        ) => Some((
            "runtimeAttachViewportDispatchRequested",
            None,
            Some(viewport.viewport_id.clone()),
            None,
        )),
        (
            "AttachViewport",
            XbxEngineControlCommandDto::AttachViewport { viewport },
            "completed",
        ) => Some((
            "runtimeAttachViewportDispatchCompleted",
            None,
            Some(viewport.viewport_id.clone()),
            None,
        )),
        (
            "StartRuntime",
            XbxEngineControlCommandDto::StartRuntime {
                session, viewport, ..
            },
            "requested",
        ) => Some((
            "runtimeStartDispatchRequested",
            Some(session.session_id.clone()),
            Some(viewport.viewport_id.clone()),
            Some(match session.target_type {
                XbxEngineTargetTypeDto::Home => "home",
                XbxEngineTargetTypeDto::Cloud => "cloud",
            }),
        )),
        (
            "StartRuntime",
            XbxEngineControlCommandDto::StartRuntime {
                session, viewport, ..
            },
            "completed",
        ) => Some((
            "runtimeStartDispatchCompleted",
            Some(session.session_id.clone()),
            Some(viewport.viewport_id.clone()),
            Some(match session.target_type {
                XbxEngineTargetTypeDto::Home => "home",
                XbxEngineTargetTypeDto::Cloud => "cloud",
            }),
        )),
        _ => None,
    }
}

fn parse_required_params<T: for<'de> Deserialize<'de>>(
    command_name: &str,
    params: Option<Value>,
) -> AppResult<T> {
    let value = params.ok_or_else(|| {
        AppError::InvalidParams(format!(
            "Missing params for xbxengine command: {command_name}"
        ))
    })?;
    serde_json::from_value(value).map_err(|error| {
        AppError::InvalidParams(format!(
            "Invalid params for xbxengine command {command_name}: {error}"
        ))
    })
}

fn to_runtime_projection(projection: StreamingRuntimeProjection) -> XbxEngineRuntimeProjectionDto {
    XbxEngineRuntimeProjectionDto {
        codec: projection
            .codec
            .map(|codec| XbxEngineRuntimeCodecPreferenceDto {
                mime_type: codec.mime_type,
                profiles: codec.profiles,
            }),
        max_video_bitrate_kbps: projection.max_video_bitrate_kbps,
        max_audio_bitrate_kbps: projection.max_audio_bitrate_kbps,
        target_video_width: projection.target_video_width,
        target_video_height: projection.target_video_height,
        force_mono_audio: projection.force_mono_audio,
        prefer_ipv6: projection.prefer_ipv6,
        bwe_mode: match projection.bwe_mode {
            crate::mods::streaming::types::StreamingBweMode::FixedRemb => "fixed-remb".to_string(),
            crate::mods::streaming::types::StreamingBweMode::ObservedRemb => {
                "observed-remb".to_string()
            }
            crate::mods::streaming::types::StreamingBweMode::Hybrid => "hybrid".to_string(),
            crate::mods::streaming::types::StreamingBweMode::TwccGcc => "twcc-gcc".to_string(),
        },
        forced_remb_kbps: projection.forced_remb_kbps,
        adaptive_remb_enabled: projection.adaptive_remb_enabled,
        remb_floor_kbps: projection.remb_floor_kbps,
        remb_ceiling_kbps: projection.remb_ceiling_kbps,
        remb_ramp_up_step_kbps: projection.remb_ramp_up_step_kbps,
        remb_ramp_down_factor: projection.remb_ramp_down_factor,
        video_pipeline: XbxEngineRuntimeVideoPipelineDto {
            feedback_interval_ms: projection.video_pipeline.feedback_interval_ms,
            nack_window_ms: projection.video_pipeline.nack_window_ms,
            nack_burst_count: projection.video_pipeline.nack_burst_count,
            nack_max_age_ms: projection.video_pipeline.nack_max_age_ms,
            nack_retry_interval_ms: projection.video_pipeline.nack_retry_interval_ms,
            nack_max_retry_count: projection.video_pipeline.nack_max_retry_count,
            jitter_buffer_min_delay_ms: projection.video_pipeline.jitter_buffer_min_delay_ms,
            jitter_buffer_max_delay_ms: projection.video_pipeline.jitter_buffer_max_delay_ms,
            jitter_buffer_max_packets: projection.video_pipeline.jitter_buffer_max_packets,
            idle_timeout_ms: projection.video_pipeline.idle_timeout_ms,
            late_frame_drop_threshold_ms: projection.video_pipeline.late_frame_drop_threshold_ms,
            backlog_drop_threshold_packets: projection
                .video_pipeline
                .backlog_drop_threshold_packets,
            jitter_early_emit_enabled: projection.video_pipeline.jitter_early_emit_enabled,
        },
        recovery: XbxEngineRuntimeRecoveryDto {
            first_frame_grace_ms: projection.recovery.first_frame_grace_ms,
            keyframe_request_stall_ms: projection.recovery.keyframe_request_stall_ms,
            keyframe_loss_burst_threshold: projection.recovery.keyframe_loss_burst_threshold,
            decoder_reset_after_keyframe_wait_ms: projection
                .recovery
                .decoder_reset_after_keyframe_wait_ms,
            decoder_reset_request_cooldown_ms: projection
                .recovery
                .decoder_reset_request_cooldown_ms,
            reconnect_stall_ms: projection.recovery.reconnect_stall_ms,
            stall_recovery_cooldown_ms: projection.recovery.stall_recovery_cooldown_ms,
        },
        polling_rate_hz: u32::from(projection.polling_rate_hz),
        vibration: projection.vibration,
    }
}

fn to_render_projection(projection: StreamingRenderProjection) -> XbxEngineRenderProjectionDto {
    XbxEngineRenderProjectionDto {
        enable_audio_control: projection.enable_audio_control,
        video_format: projection.video_format,
        display_options: XbxEngineDisplayOptionsDto {
            sharpness: projection.display_options.sharpness as f32,
            saturation: projection.display_options.saturation as f32,
            contrast: projection.display_options.contrast as f32,
            brightness: projection.display_options.brightness as f32,
        },
    }
}
