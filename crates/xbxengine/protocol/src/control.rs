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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineRuntimeProjectionDto {
    pub codec: Option<XbxEngineRuntimeCodecPreferenceDto>,
    pub max_video_bitrate_kbps: Option<u32>,
    pub max_audio_bitrate_kbps: Option<u32>,
    pub target_video_width: u32,
    pub target_video_height: u32,
    pub force_mono_audio: bool,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum XbxStreamingModeDto {
    #[default]
    CloudGaming,
    LocalHost,
    CloudHost,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum XbxEngineControlCommandDto {
    StartRuntime {
        session: XbxEngineSessionDto,
        viewport: XbxEngineViewportDto,
        audio_volume: f32,
        mode: Option<XbxStreamingModeDto>,
        runtime: Option<XbxEngineRuntimeProjectionDto>,
        render: Option<XbxEngineRenderProjectionDto>,
    },
    StopRuntime,
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
