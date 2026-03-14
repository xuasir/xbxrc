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
pub struct XbxEnginePacketGapObservationDto {
    pub observation_id: u64,
    pub expected_sequence: u16,
    pub received_sequence: u16,
    pub missing_count: u16,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineFrameDropObservationDto {
    pub observation_id: u64,
    pub reason: String,
    pub observed_at_ms: f64,
    pub width: u32,
    pub height: u32,
    pub is_keyframe: bool,
    pub queue_depth: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineNackObservationDto {
    pub observation_id: u64,
    pub action: String,
    pub first_sequence: u16,
    pub last_sequence: u16,
    pub packet_count: u16,
    pub retry_count: u8,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineVideoEscalationObservationDto {
    pub observation_id: u64,
    pub reason: String,
    pub action: String,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineVideoBweObservationDto {
    pub observation_id: u64,
    pub mode: String,
    pub decision_reason: String,
    pub target_remb_kbps: u32,
    pub observed_remb_kbps: Option<u32>,
    pub actual_video_bitrate_kbps: f64,
    pub loss_ratio: f64,
    pub rtt_ms: Option<f64>,
    pub transport_path: Option<String>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineStatsDto {
    pub resolution: String,
    pub rtt: String,
    pub fps: f64,
    pub inbound_video_fps: Option<f64>,
    pub decode_fps: Option<f64>,
    pub present_fps: Option<f64>,
    pub pl: String,
    pub fl: String,
    pub jit: String,
    pub br: String,
    pub decode: String,
    pub transport_path: Option<String>,
    pub transport_state: Option<String>,
    pub video_rtt_source: Option<String>,
    pub video_remb_bps: Option<u32>,
    pub inbound_bitrate_kbps: Option<f64>,
    pub inbound_video_bitrate_kbps: Option<f64>,
    pub inbound_audio_bitrate_kbps: Option<f64>,
    pub inbound_bytes_total: Option<u64>,
    pub inbound_video_bytes_total: Option<u64>,
    pub inbound_audio_bytes_total: Option<u64>,
    pub inbound_video_packet_count_total: Option<u64>,
    pub video_decoder_reset_count: Option<u64>,
    pub video_decoder_stalled: Option<bool>,
    pub video_renderer_stalled: Option<bool>,
    pub packet_age_ms: Option<f64>,
    pub decode_age_ms: Option<f64>,
    pub present_age_ms: Option<f64>,
    pub packet_to_decode_ms: Option<f64>,
    pub decode_to_present_ms: Option<f64>,
    pub packet_to_present_ms: Option<f64>,
    pub video_decode_input_drop_count_total: Option<u64>,
    pub video_decode_output_drop_count_total: Option<u64>,
    pub video_pacer_submit_count_total: Option<u64>,
    pub video_pacer_drop_count_total: Option<u64>,
    pub video_renderer_submit_count_total: Option<u64>,
    pub video_renderer_drop_count_total: Option<u64>,
    pub video_present_overwrite_count_total: Option<u64>,
    pub video_present_submit_count_total: Option<u64>,
    pub recovery_keyframe_request_count: Option<u64>,
    pub recovery_decoder_reset_count: Option<u64>,
    pub recovery_reconnect_count: Option<u64>,
    pub last_recovery_action: Option<String>,
    pub last_recovery_action_at_ms: Option<f64>,
    pub last_recovery_reason: Option<String>,
    pub latest_video_packet_gap: Option<XbxEnginePacketGapObservationDto>,
    pub latest_video_frame_drop: Option<XbxEngineFrameDropObservationDto>,
    pub latest_video_nack_observation: Option<XbxEngineNackObservationDto>,
    pub latest_video_escalation_observation: Option<XbxEngineVideoEscalationObservationDto>,
    pub latest_video_bwe_observation: Option<XbxEngineVideoBweObservationDto>,
}
