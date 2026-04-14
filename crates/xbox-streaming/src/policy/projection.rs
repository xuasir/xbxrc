use serde::{Deserialize, Serialize};

use crate::policy::context::Context;
use crate::policy::input::compiler::interpret_input_capabilities;
use crate::policy::input::InputCapabilitySource;
use crate::policy::input::SupportedInput;
use crate::policy::negotiation::Codec;
use crate::policy::plan::Plan;
use crate::policy::render::RenderDisplayOptions;
use crate::policy::runtime::{RuntimeBweMode, RuntimeMode};
use crate::policy::session::SessionSchedulePlan;
use crate::policy::types::{Owner, Region, TurnServer, TurnSource};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeCodecProjection {
    pub mime_type: String,
    pub profiles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeVideoPipelineProjection {
    pub feedback_interval_ms: u64,
    pub nack_window_ms: u64,
    pub nack_burst_count: u16,
    pub nack_max_age_ms: u64,
    pub nack_retry_interval_ms: u64,
    pub nack_max_retry_count: u8,
    pub jitter_buffer_min_delay_ms: u64,
    pub jitter_buffer_max_delay_ms: u64,
    pub jitter_buffer_max_packets: u16,
    pub idle_timeout_ms: u64,
    pub late_frame_drop_threshold_ms: u64,
    pub backlog_drop_threshold_packets: u16,
    pub jitter_early_emit_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeRecoveryProjection {
    pub first_frame_grace_ms: u64,
    pub keyframe_request_stall_ms: u64,
    pub keyframe_loss_burst_threshold: u8,
    pub decoder_reset_after_keyframe_wait_ms: u64,
    pub decoder_reset_request_cooldown_ms: u64,
    pub reconnect_stall_ms: u64,
    pub stall_recovery_cooldown_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RenderDisplayOptionsProjection {
    pub sharpness: i16,
    pub saturation: i16,
    pub contrast: i16,
    pub brightness: i16,
}

/// session 元数据投影：给 UI/HUD 提供稳定上下文，不暴露完整 plan。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionMetadataProjection {
    pub server_base_url: String,
    pub region: Option<SessionRegionProjection>,
    pub turn_source: TurnSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionRegionProjection {
    pub name: String,
    pub short_name: Option<String>,
    pub display_name: Option<String>,
    pub continent: Option<String>,
}

/// session 输入能力快照：只暴露事实与当前 plan 结果，不在 renderer 重新解释 context。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionCapabilitiesProjection {
    pub supported_inputs: Vec<String>,
    pub title_supports_mkb: bool,
    pub title_supports_touch: bool,
    pub title_supports_native_touch: bool,
    pub input_config_resolved: bool,
    pub input_config_supports_mkb: bool,
    pub input_config_supports_touch: bool,
    pub input_config_supports_native_touch: bool,
    pub effective_capability_source: String,
    pub effective_title_supports_mkb: bool,
    pub effective_title_supports_touch: bool,
    pub effective_title_supports_native_touch: bool,
    pub runtime_supports_native_mkb: bool,
    pub runtime_supports_touch_surface: bool,
    pub remote_play_configuration_resolved: bool,
    pub remote_play_remote_management_enabled: bool,
    pub remote_play_console_streaming_enabled: bool,
    pub effective_remote_play_capability_source: String,
    pub effective_remote_play_allows_streaming: bool,
    pub remote_play_console_addrs_count: u32,
    pub input_mode: String,
    pub touch_enabled: bool,
    pub microphone_start_with_session: bool,
}

/// runtime 执行层可消费的最小投影，避免 crate 外再次解释策略。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimePlanProjection {
    pub mode: RuntimeMode,
    pub transport: Owner,
    pub decode: Owner,
    pub render: Owner,
    pub input: Owner,
    pub microphone: Owner,
    pub target_video_width: u32,
    pub target_video_height: u32,
    /// 运行时启动后是否按策略自动尝试开麦。
    pub microphone_start_with_session: bool,
    pub turn_server: Option<TurnServer>,
    pub codec: Option<RuntimeCodecProjection>,
    pub max_video_bitrate_kbps: Option<u32>,
    pub max_audio_bitrate_kbps: Option<u32>,
    pub force_mono_audio: bool,
    pub prefer_ipv6: bool,
    pub bwe_mode: RuntimeBweMode,
    pub forced_remb_kbps: Option<u32>,
    pub adaptive_remb_enabled: bool,
    pub remb_floor_kbps: u32,
    pub remb_ceiling_kbps: u32,
    pub remb_ramp_up_step_kbps: u32,
    pub remb_ramp_down_factor: u16,
    pub video_pipeline: RuntimeVideoPipelineProjection,
    pub recovery: RuntimeRecoveryProjection,
    pub polling_rate_hz: u16,
    pub vibration: bool,
}

/// render 执行层可消费的最小投影，避免 renderer 继续读取 raw config。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RenderPlanProjection {
    pub enable_audio_control: bool,
    pub video_format: Option<String>,
    pub display_options: RenderDisplayOptionsProjection,
}

/// session 编排可消费的调度投影，避免 adapter/runtime 读取完整 plan。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionSchedulePlanProjection {
    pub monitor_interval_ms: u64,
    pub keepalive_interval_ms: u64,
    pub offer_poll_interval_ms: u64,
    pub ice_poll_interval_ms: u64,
    pub startup_timeout_ms: u64,
    pub ready_timeout_ms: u64,
    pub retry_backoff_ms: Vec<u64>,
    pub wake_console: bool,
    pub require_console_ready: bool,
}

pub fn project_runtime_plan(plan: &Plan) -> RuntimePlanProjection {
    RuntimePlanProjection {
        mode: plan.runtime.mode,
        transport: plan.runtime.transport,
        decode: plan.runtime.decode,
        render: plan.runtime.render,
        input: plan.runtime.input,
        microphone: plan.runtime.microphone,
        target_video_width: plan.session.device.max_width,
        target_video_height: plan.session.device.max_height,
        microphone_start_with_session: plan.input.microphone_on_play,
        turn_server: plan.runtime.turn.resolved.clone(),
        codec: plan.negotiation.codec.as_ref().map(project_codec),
        max_video_bitrate_kbps: plan.negotiation.video_bitrate_kbps,
        max_audio_bitrate_kbps: plan.negotiation.audio_bitrate_kbps,
        force_mono_audio: !plan.negotiation.stereo_audio,
        prefer_ipv6: plan.negotiation.prefer_ipv6,
        bwe_mode: plan.runtime.bwe_mode,
        forced_remb_kbps: plan.runtime.forced_remb_kbps,
        adaptive_remb_enabled: plan.runtime.adaptive_remb_enabled,
        remb_floor_kbps: plan.runtime.remb_floor_kbps,
        remb_ceiling_kbps: plan.runtime.remb_ceiling_kbps,
        remb_ramp_up_step_kbps: plan.runtime.remb_ramp_up_step_kbps,
        remb_ramp_down_factor: plan.runtime.remb_ramp_down_factor,
        video_pipeline: project_video_pipeline(&plan.runtime.video_pipeline),
        recovery: project_recovery(&plan.runtime.recovery),
        polling_rate_hz: plan.input.polling_rate_hz,
        vibration: plan.input.vibration,
    }
}

pub fn project_render_plan(plan: &Plan) -> RenderPlanProjection {
    RenderPlanProjection {
        enable_audio_control: plan.render.enable_audio_control,
        video_format: plan.render.video_format.clone(),
        display_options: project_display_options(&plan.render.display_options),
    }
}

pub fn project_session_metadata(plan: &Plan) -> SessionMetadataProjection {
    SessionMetadataProjection {
        server_base_url: plan.session.base_url.clone(),
        region: plan.session.region.as_ref().map(project_region),
        turn_source: plan.runtime.turn.source,
    }
}

pub fn project_session_capabilities(
    context: &Context,
    plan: &Plan,
) -> SessionCapabilitiesProjection {
    let effective = interpret_input_capabilities(context);
    SessionCapabilitiesProjection {
        supported_inputs: context
            .input
            .supported_inputs
            .iter()
            .map(project_supported_input)
            .collect(),
        title_supports_mkb: context.input.has_mkb,
        title_supports_touch: context.input.has_touch,
        title_supports_native_touch: context.input.has_native_touch,
        input_config_resolved: context.input_capability.input_config_resolved,
        input_config_supports_mkb: context.input_capability.input_config_supports_mkb,
        input_config_supports_touch: context.input_capability.input_config_supports_touch,
        input_config_supports_native_touch: context
            .input_capability
            .input_config_supports_native_touch,
        effective_capability_source: project_input_capability_source(effective.source),
        effective_title_supports_mkb: effective.title_supports_mkb,
        effective_title_supports_touch: effective.title_supports_touch,
        effective_title_supports_native_touch: effective.title_supports_native_touch,
        runtime_supports_native_mkb: context.runtime.native_mkb,
        runtime_supports_touch_surface: context.runtime.touch_surface,
        remote_play_configuration_resolved: context.remote_play.configuration_resolved,
        remote_play_remote_management_enabled: context
            .remote_play
            .remote_management_enabled
            .unwrap_or(false),
        remote_play_console_streaming_enabled: context
            .remote_play
            .console_streaming_enabled
            .unwrap_or(false),
        effective_remote_play_capability_source: if context.remote_play.configuration_resolved {
            "configuration".to_string()
        } else {
            "fallback".to_string()
        },
        effective_remote_play_allows_streaming: resolve_remote_play_streaming_allowed(context),
        remote_play_console_addrs_count: context.remote_play.console_addrs.len() as u32,
        input_mode: format!("{:?}", plan.input.mode),
        touch_enabled: plan.input.touch,
        microphone_start_with_session: plan.input.microphone_on_play,
    }
}

pub fn project_session_schedule_plan(plan: &Plan) -> SessionSchedulePlanProjection {
    project_session_schedule(&plan.session.schedule)
}

fn project_codec(codec: &Codec) -> RuntimeCodecProjection {
    RuntimeCodecProjection {
        mime_type: codec.mime_type.clone(),
        profiles: codec.profiles.clone(),
    }
}

fn project_display_options(options: &RenderDisplayOptions) -> RenderDisplayOptionsProjection {
    RenderDisplayOptionsProjection {
        sharpness: options.sharpness,
        saturation: options.saturation,
        contrast: options.contrast,
        brightness: options.brightness,
    }
}

fn project_video_pipeline(
    plan: &crate::policy::runtime::RuntimeVideoPipelinePlan,
) -> RuntimeVideoPipelineProjection {
    RuntimeVideoPipelineProjection {
        feedback_interval_ms: plan.feedback_interval_ms,
        nack_window_ms: plan.nack_window_ms,
        nack_burst_count: plan.nack_burst_count,
        nack_max_age_ms: plan.nack_max_age_ms,
        nack_retry_interval_ms: plan.nack_retry_interval_ms,
        nack_max_retry_count: plan.nack_max_retry_count,
        jitter_buffer_min_delay_ms: plan.jitter_buffer_min_delay_ms,
        jitter_buffer_max_delay_ms: plan.jitter_buffer_max_delay_ms,
        jitter_buffer_max_packets: plan.jitter_buffer_max_packets,
        idle_timeout_ms: plan.idle_timeout_ms,
        late_frame_drop_threshold_ms: plan.late_frame_drop_threshold_ms,
        backlog_drop_threshold_packets: plan.backlog_drop_threshold_packets,
        jitter_early_emit_enabled: plan.jitter_early_emit_enabled,
    }
}

fn project_recovery(
    plan: &crate::policy::runtime::RuntimeRecoveryPlan,
) -> RuntimeRecoveryProjection {
    RuntimeRecoveryProjection {
        first_frame_grace_ms: plan.first_frame_grace_ms,
        keyframe_request_stall_ms: plan.keyframe_request_stall_ms,
        keyframe_loss_burst_threshold: plan.keyframe_loss_burst_threshold,
        decoder_reset_after_keyframe_wait_ms: plan.decoder_reset_after_keyframe_wait_ms,
        decoder_reset_request_cooldown_ms: plan.decoder_reset_request_cooldown_ms,
        reconnect_stall_ms: plan.reconnect_stall_ms,
        stall_recovery_cooldown_ms: plan.stall_recovery_cooldown_ms,
    }
}

fn project_region(region: &Region) -> SessionRegionProjection {
    SessionRegionProjection {
        name: region.name.clone(),
        short_name: region.short_name.clone(),
        display_name: region.display_name.clone(),
        continent: region.continent.clone(),
    }
}

fn project_supported_input(input: &SupportedInput) -> String {
    match input {
        SupportedInput::Gamepad => "gamepad".to_string(),
        SupportedInput::Mouse => "mouse".to_string(),
        SupportedInput::Keyboard => "keyboard".to_string(),
        SupportedInput::NativeTouch => "nativeTouch".to_string(),
        SupportedInput::GenericTouch => "genericTouch".to_string(),
        SupportedInput::CustomTouchOverlay => "customTouchOverlay".to_string(),
        SupportedInput::Mkb => "mkb".to_string(),
        SupportedInput::Unknown(value) => value.clone(),
    }
}

fn project_input_capability_source(source: InputCapabilitySource) -> String {
    match source {
        InputCapabilitySource::Fallback => "fallback".to_string(),
        InputCapabilitySource::InputConfig => "inputConfig".to_string(),
    }
}

fn resolve_remote_play_streaming_allowed(context: &Context) -> bool {
    if context.remote_play.configuration_resolved {
        return context
            .remote_play
            .console_streaming_enabled
            .unwrap_or(false);
    }

    !context.remote_play.console_addrs.is_empty()
}

fn project_session_schedule(schedule: &SessionSchedulePlan) -> SessionSchedulePlanProjection {
    SessionSchedulePlanProjection {
        monitor_interval_ms: schedule.monitor_interval_ms,
        keepalive_interval_ms: schedule.keepalive_interval_ms,
        offer_poll_interval_ms: schedule.offer_poll_interval_ms,
        ice_poll_interval_ms: schedule.ice_poll_interval_ms,
        startup_timeout_ms: schedule.startup_timeout_ms,
        ready_timeout_ms: schedule.ready_timeout_ms,
        retry_backoff_ms: schedule.retry_backoff_ms.clone(),
        wake_console: schedule.wake_console,
        require_console_ready: schedule.require_console_ready,
    }
}
