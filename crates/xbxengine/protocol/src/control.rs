use crate::{XbxEngineSessionDto, XbxEngineViewportDto};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum XbxEngineReconnectReasonDto {
    NetworkLost,
    IceFailed,
    MediaStalled,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineDisplayOptionsDto {
    pub sharpness: f32,
    pub saturation: f32,
    pub contrast: f32,
    pub brightness: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineDisplayStateDto {
    pub display_options: XbxEngineDisplayOptionsDto,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct XbxEngineRuntimeCodecPreferenceDto {
    pub mime_type: String,
    pub profiles: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct XbxEngineRuntimeVideoPipelineDto {
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
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct XbxEngineRuntimeRecoveryDto {
    pub first_frame_grace_ms: u64,
    pub keyframe_request_stall_ms: u64,
    pub keyframe_loss_burst_threshold: u8,
    pub decoder_reset_after_keyframe_wait_ms: u64,
    pub decoder_reset_request_cooldown_ms: u64,
    pub reconnect_stall_ms: u64,
    pub stall_recovery_cooldown_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineRuntimeProjectionDto {
    pub codec: Option<XbxEngineRuntimeCodecPreferenceDto>,
    pub max_video_bitrate_kbps: Option<u32>,
    pub max_audio_bitrate_kbps: Option<u32>,
    pub target_video_width: u32,
    pub target_video_height: u32,
    pub force_mono_audio: bool,
    #[serde(default)]
    pub prefer_ipv6: bool,
    pub bwe_mode: String,
    pub forced_remb_kbps: Option<u32>,
    pub adaptive_remb_enabled: bool,
    pub remb_floor_kbps: u32,
    pub remb_ceiling_kbps: u32,
    pub remb_ramp_up_step_kbps: u32,
    pub remb_ramp_down_factor: u16,
    pub video_pipeline: XbxEngineRuntimeVideoPipelineDto,
    pub recovery: XbxEngineRuntimeRecoveryDto,
    pub polling_rate_hz: u32,
    pub vibration: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineRenderProjectionDto {
    pub enable_audio_control: bool,
    pub video_format: Option<String>,
    pub display_options: XbxEngineDisplayOptionsDto,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum XbxEngineInputEventDto {
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum XbxEngineControlCommandDto {
    StartRuntime {
        session: XbxEngineSessionDto,
        viewport: XbxEngineViewportDto,
        audio_volume: f32,
        runtime: Option<XbxEngineRuntimeProjectionDto>,
        render: Option<XbxEngineRenderProjectionDto>,
    },
    StopRuntime {
        reason: Option<String>,
    },
    RequestReconnect {
        reason: XbxEngineReconnectReasonDto,
    },
    AttachViewport {
        viewport: XbxEngineViewportDto,
    },
    DetachViewport,
    ApplyDisplayState {
        state: XbxEngineDisplayStateDto,
    },
    SetAudioVolume {
        value: f32,
    },
    StartMicrophone,
    StopMicrophone,
    PressControllerButton {
        button: String,
        duration_ms: u64,
    },
    SetKeyboardPointerEnabled {
        enabled: bool,
    },
    PushKeyboardPointerInput {
        event: XbxEngineInputEventDto,
    },
}
