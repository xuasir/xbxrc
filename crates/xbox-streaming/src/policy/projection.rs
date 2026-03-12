use serde::{Deserialize, Serialize};

use crate::policy::types::{Owner, TurnServer};
use crate::policy::negotiation::Codec;
use crate::policy::plan::Plan;
use crate::policy::render::RenderDisplayOptions;
use crate::policy::runtime::RuntimeMode;
use crate::policy::session::SessionSchedulePlan;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeCodecProjection {
    pub mime_type: String,
    pub profiles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RenderDisplayOptionsProjection {
    pub sharpness: i16,
    pub saturation: i16,
    pub contrast: i16,
    pub brightness: i16,
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
    pub turn_server: Option<TurnServer>,
    pub codec: Option<RuntimeCodecProjection>,
    pub max_video_bitrate_kbps: Option<u32>,
    pub max_audio_bitrate_kbps: Option<u32>,
    pub force_mono_audio: bool,
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
        turn_server: plan.runtime.turn.resolved.clone(),
        codec: plan.negotiation.codec.as_ref().map(project_codec),
        max_video_bitrate_kbps: plan.negotiation.video_bitrate_kbps,
        max_audio_bitrate_kbps: plan.negotiation.audio_bitrate_kbps,
        force_mono_audio: !plan.negotiation.stereo_audio,
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
