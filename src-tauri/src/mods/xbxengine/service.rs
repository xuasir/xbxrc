use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use tauri::AppHandle;
use xbxengine_protocol::{
    XbxEngineControlCommandDto, XbxEngineDisplayOptionsDto, XbxEngineDisplayStateDto,
    XbxEngineInputEventDto, XbxEngineReconnectReasonDto, XbxEngineRenderProjectionDto,
    XbxEngineRuntimeCodecPreferenceDto, XbxEngineRuntimeProjectionDto, XbxEngineRuntimeRecoveryDto,
    XbxEngineRuntimeVideoPipelineDto, XbxEngineSessionDto, XbxEngineTargetTypeDto,
    XbxEngineTurnServerDto, XbxEngineViewportDto,
};

use crate::error::{AppError, AppResult};
use crate::mods::native_video::NativeVideoRegistryRef;
use crate::mods::runtime_trace::RuntimeTraceRecorderRef;
use crate::mods::streaming::{
    StreamingRenderProjection, StreamingRuntimeProjection, StreamingTurnServerConfig,
};
use crate::mods::xbxengine::runtime_state::XbxEngineRuntimeState;
use crate::mods::xbxengine::XbxEngineProvider;

const XBXENGINE_TICK_INTERVAL_MS: u64 = 16;

pub struct XbxEngineService {
    runtime_state: Arc<XbxEngineRuntimeState>,
    last_runtime_event: Arc<StdMutex<Option<Value>>>,
}

impl XbxEngineService {
    pub fn new(
        app_handle: AppHandle,
        last_runtime_event: Arc<StdMutex<Option<Value>>>,
        native_video: NativeVideoRegistryRef,
        runtime_trace: RuntimeTraceRecorderRef,
    ) -> Self {
        Self {
            runtime_state: Arc::new(XbxEngineRuntimeState::new(
                app_handle,
                last_runtime_event.clone(),
                native_video,
                runtime_trace,
            )),
            last_runtime_event,
        }
    }
}

#[async_trait]
impl XbxEngineProvider for XbxEngineService {
    async fn dispatch_control(&self, command_name: &str, params: Option<Value>) -> AppResult<()> {
        let command = parse_control_command(command_name, params)?;
        let runtime_state = self.runtime_state.clone();
        tauri::async_runtime::spawn_blocking(move || runtime_state.apply_control(command))
            .await
            .map_err(|error| {
                AppError::XbxEngine(format!("xbxengine control task join failed: {error}"))
            })?
            .map_err(|error| AppError::XbxEngine(error.to_string()))
    }

    async fn snapshot_stats(&self) -> AppResult<Value> {
        self.runtime_state.snapshot_stats()
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
        true
    }

    fn bind_tasks(&self, is_quitting: Arc<AtomicBool>) {
        let runtime_state = self.runtime_state.clone();
        tauri::async_runtime::spawn(async move {
            let mut interval =
                tokio::time::interval(Duration::from_millis(XBXENGINE_TICK_INTERVAL_MS));
            while !is_quitting.load(Ordering::Relaxed) {
                interval.tick().await;
                let state = runtime_state.clone();
                let result = tauri::async_runtime::spawn_blocking(move || state.tick()).await;
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        log::warn!("xbxengine tick failed: {}", error);
                    }
                    Err(error) => {
                        log::warn!("xbxengine tick task join failed: {}", error);
                    }
                }
            }
        });
    }

    async fn shutdown(&self) {
        let _ = self.dispatch_control("StopRuntime", None).await;
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartRuntimeParams {
    session: StartRuntimeSessionParams,
    viewport: AttachViewportParams,
    runtime: StreamingRuntimeProjection,
    render: StreamingRenderProjection,
    audio_volume: f32,
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
struct PressControllerButtonParams {
    button: String,
    duration_ms: u64,
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
        "StopRuntime" => Ok(XbxEngineControlCommandDto::StopRuntime),
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
                    display_options: XbxEngineDisplayOptionsDto {
                        sharpness: params.state.display_options.sharpness,
                        saturation: params.state.display_options.saturation,
                        contrast: params.state.display_options.contrast,
                        brightness: params.state.display_options.brightness,
                    },
                },
            })
        }
        "PressControllerButton" => {
            let params: PressControllerButtonParams = parse_required_params(command_name, params)?;
            Ok(XbxEngineControlCommandDto::PressControllerButton {
                button: params.button,
                duration_ms: params.duration_ms,
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
