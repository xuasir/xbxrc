use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum XbxEngineTargetTypeDto {
    Home,
    Cloud,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct XbxEngineTurnServerDto {
    pub url: String,
    pub username: String,
    pub credential: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct XbxEngineSessionDto {
    pub session_id: String,
    pub target_type: XbxEngineTargetTypeDto,
    pub turn_server: Option<XbxEngineTurnServerDto>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct XbxEngineViewportDto {
    pub viewport_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum XbxEngineRuntimePhaseDto {
    Binding,
    ExchangingOffer,
    GatheringIce,
    ExchangingIce,
    Connecting,
    Reconnecting,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum XbxEngineTransportStateDto {
    New,
    Connecting,
    Connected,
    Disconnected,
    Failed,
    Closed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum XbxEngineRuntimeEventDto {
    RuntimePhaseChanged {
        phase: XbxEngineRuntimePhaseDto,
    },
    TransportConnectionStateChanged {
        state: XbxEngineTransportStateDto,
    },
    ChatStateChanged {
        capturing: bool,
        paused: bool,
    },
    MediaVideoReady {
        width: u32,
        height: u32,
    },
    MediaSurfaceReady {
        surface_id: String,
    },
    StatsVideoFrameProcessed {
        first_frame_packet_arrival_time_ms: f64,
        frame_decoded_time_ms: f64,
        frame_rendered_time_ms: f64,
    },
    DiagnosticsPulse {
        window_ms: f64,
        frames_in_window: u64,
        fps: f64,
        render_idle_ms: Option<f64>,
        inbound_kbps: f64,
        inbound_video_kbps: f64,
        inbound_primary_video_kbps: f64,
        inbound_audio_kbps: f64,
        inbound_video_packets_in_window: u64,
        inbound_video_loss_ratio_1s: f64,
        inbound_video_loss_ratio_5s: f64,
        video_rtt_ms: Option<f64>,
        video_rtt_source: Option<String>,
        video_nack_recovery_rtt_ms: Option<f64>,
        video_remb_bps: Option<u32>,
        inbound_video_jitter_ms: Option<f64>,
        video_loss_finalized_packets_in_window: u64,
        video_loss_recovered_packets_in_window: u64,
        video_loss_late_recovered_packets_in_window: u64,
        video_width: Option<u32>,
        video_height: Option<u32>,
        transport_state: XbxEngineTransportStateDto,
    },
    ErrorReported {
        code: String,
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineStatsDto {
    pub resolution: String,
    pub rtt: String,
    pub fps: f64,
    pub pl: String,
    pub fl: String,
    pub jit: String,
    pub br: String,
    pub decode: String,
}
