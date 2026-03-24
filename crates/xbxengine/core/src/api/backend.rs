use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use ohmygamepad_protocol::OhMyGamepadRumbleRequestDto;
use xbxengine_protocol::{
    XbxEngineDisplayStateDto, XbxEngineIceCandidateDto, XbxEngineInputEventDto,
    XbxEngineSessionDto, XbxEngineTargetTypeDto, XbxEngineTransportStateDto, XbxEngineViewportDto,
};

use crate::api::input::{NoopXbxEngineInputBackend, XbxEngineInputBackend, XbxEngineInputStatus};
use crate::api::runtime::{XbxEngineRuntimeConfig, XbxEngineRuntimeError};

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineMediaNegotiationRequest {
    pub session: XbxEngineSessionDto,
    pub viewport: XbxEngineViewportDto,
    pub restart: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineMediaNegotiation {
    pub local_offer_sdp: String,
    pub local_candidates: Vec<XbxEngineIceCandidateDto>,
    pub surface_id: String,
    pub video_width: u32,
    pub video_height: u32,
    pub first_frame_packet_arrival_time_ms: Option<f64>,
    pub frame_decoded_time_ms: Option<f64>,
    pub frame_rendered_time_ms: Option<f64>,
    pub input_status: XbxEngineInputStatus,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineVideoFrameStats {
    pub width: u32,
    pub height: u32,
    pub frame_seq: u64,
    pub fps: f64,
    pub rendered_at_ms: f64,
}

pub type CFDictionaryRef = *const std::ffi::c_void;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MacOsVideoColorMatrix {
    #[default]
    Bt709,
    Bt601,
    Smpte240M,
    Bt2020,
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MacOsVideoColorPrimaries {
    #[default]
    Bt709,
    P3D65,
    Bt2020,
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MacOsVideoTransferFunction {
    #[default]
    Bt709,
    Srgb,
    Linear,
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MacOsVideoColorRange {
    #[default]
    Video,
    Full,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MacOsVideoChromaLocation {
    #[default]
    Center,
    Left,
    TopLeft,
    Unknown,
}

pub struct MacOsCVPixelBufferDescriptor {
    pub ptr: *mut std::ffi::c_void,
    pub color_matrix: MacOsVideoColorMatrix,
    pub color_primaries: MacOsVideoColorPrimaries,
    pub transfer_function: MacOsVideoTransferFunction,
    pub color_range: MacOsVideoColorRange,
    pub chroma_location: MacOsVideoChromaLocation,
    pub drop_fn: Option<Box<dyn FnOnce(*mut std::ffi::c_void) + Send + Sync>>,
}

impl std::fmt::Debug for MacOsCVPixelBufferDescriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MacOsCVPixelBufferDescriptor")
            .field("ptr", &self.ptr)
            .field("color_matrix", &self.color_matrix)
            .field("color_primaries", &self.color_primaries)
            .field("transfer_function", &self.transfer_function)
            .field("color_range", &self.color_range)
            .field("chroma_location", &self.chroma_location)
            .field(
                "drop_fn",
                &if self.drop_fn.is_some() {
                    "Some(<closure>)"
                } else {
                    "None"
                },
            )
            .finish()
    }
}

impl Drop for MacOsCVPixelBufferDescriptor {
    fn drop(&mut self) {
        if let Some(drop_fn) = self.drop_fn.take() {
            drop_fn(self.ptr);
        }
    }
}

unsafe impl Send for MacOsCVPixelBufferDescriptor {}
unsafe impl Sync for MacOsCVPixelBufferDescriptor {}

#[derive(Clone, Debug)]
pub enum XbxEngineRenderPixelData {
    Rgba {
        bytes: Arc<[u8]>,
    },
    Bgra {
        bytes: Arc<[u8]>,
    },
    Nv12 {
        y_plane: Arc<[u8]>,
        uv_plane: Arc<[u8]>,
        y_stride: u32,
        uv_stride: u32,
    },
    Descriptor {
        handle: Arc<dyn std::any::Any + Send + Sync>,
    },
}

impl PartialEq for XbxEngineRenderPixelData {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Rgba { bytes: l }, Self::Rgba { bytes: r }) => l == r,
            (Self::Bgra { bytes: l }, Self::Bgra { bytes: r }) => l == r,
            (
                Self::Nv12 {
                    y_plane: ly,
                    uv_plane: luv,
                    y_stride: lys,
                    uv_stride: luvs,
                },
                Self::Nv12 {
                    y_plane: ry,
                    uv_plane: ruv,
                    y_stride: rys,
                    uv_stride: ruvs,
                },
            ) => ly == ry && luv == ruv && lys == rys && luvs == ruvs,
            (Self::Descriptor { handle: l }, Self::Descriptor { handle: r }) => Arc::ptr_eq(l, r),
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineRenderFrame {
    pub width: u32,
    pub height: u32,
    pub frame_seq: u64,
    pub rendered_at_ms: f64,
    pub pixel_data: XbxEngineRenderPixelData,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineVideoPacketGapObservation {
    pub observation_id: u64,
    pub expected_sequence: u16,
    pub received_sequence: u16,
    pub missing_count: u16,
    pub source: String,
    pub frame_rtp_timestamp: Option<u32>,
    pub frame_packet_count: Option<u16>,
    pub frame_missing_count: Option<u16>,
    pub frame_is_keyframe: Option<bool>,
    pub frame_importance: Option<String>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineVideoFrameDropObservation {
    pub observation_id: u64,
    pub reason: String,
    pub observed_at_ms: f64,
    pub width: u32,
    pub height: u32,
    pub is_keyframe: bool,
    pub queue_depth: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineVideoNackObservation {
    pub observation_id: u64,
    pub action: String,
    pub source: String,
    pub first_sequence: u16,
    pub last_sequence: u16,
    pub packet_count: u16,
    pub retry_count: u8,
    pub frame_rtp_timestamp: Option<u32>,
    pub frame_is_keyframe: Option<bool>,
    pub frame_importance: Option<String>,
    pub deadline_at_ms: Option<f64>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineVideoEscalationObservation {
    pub observation_id: u64,
    pub reason: String,
    pub action: String,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XbxEnginePendingRuntimeRecoveryAction {
    RequestReconnectCandidate { observation_id: u64, reason: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineVideoBweObservation {
    pub observation_id: u64,
    pub mode: String,
    pub decision_reason: String,
    pub target_remb_kbps: u32,
    pub observed_remb_kbps: Option<u32>,
    pub actual_video_bitrate_kbps: f64,
    pub loss_ratio: f64,
    pub rtt_ms: Option<f64>,
    pub transport_path: Option<String>,
    pub twcc_feedback_interval_ms: Option<f64>,
    pub twcc_observed_packet_count: Option<u16>,
    pub twcc_covered_sequence_span: Option<u16>,
    pub twcc_receive_bitrate_kbps: Option<f64>,
    pub twcc_delivery_ratio: Option<f64>,
    pub twcc_loss_ratio: Option<f64>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineVideoTwccObservation {
    pub observation_id: u64,
    pub feedback_packet_count: u16,
    pub covered_sequence_start: u16,
    pub covered_sequence_end: u16,
    pub covered_sequence_span: u16,
    pub observed_packet_count: u16,
    pub observed_byte_count: u64,
    pub feedback_interval_ms: Option<f64>,
    pub arrival_span_ms: Option<f64>,
    pub receive_bitrate_kbps: Option<f64>,
    pub delivery_ratio: f64,
    pub packet_loss_ratio: f64,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineVideoTrackStatus {
    pub state: String,
    pub video_width: Option<u32>,
    pub video_height: Option<u32>,
    pub mime_type: Option<String>,
    pub transport_state: XbxEngineTransportStateDto,
    pub video_bytes_total: u64,
    pub video_packet_count_total: u64,
    pub audio_bytes_total: u64,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineVideoRepairProbeObservation {
    pub observation_id: u64,
    pub phase: String,
    pub classification: String,
    pub stream_id: String,
    pub stream_ssrc: u32,
    pub mime_type: String,
    pub payload_type: u8,
    pub clock_rate: u32,
    pub associated_ssrc: Option<u32>,
    pub associated_payload_type: Option<u8>,
    pub stream_packet_count: u64,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineVideoRtxReinjectObservation {
    pub stage: String,
    pub primary_ssrc: u32,
    pub repair_ssrc: u32,
    pub sequence_number: u16,
    pub rtp_timestamp: u32,
    pub pending_queue_len: usize,
    pub native_sequence_number: Option<u16>,
    pub matched_head_gap: bool,
    pub matched_nack_range: bool,
    pub matched_pending_gap: bool,
    pub matched_gap_sequence: Option<u16>,
    pub matched_nack_first_sequence: Option<u16>,
    pub matched_nack_last_sequence: Option<u16>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineDataChannelMessageCatalogObservation {
    pub observation_id: u64,
    pub direction: String,
    pub channel: String,
    pub kind_type: Option<String>,
    pub kind_message: Option<String>,
    pub target: Option<String>,
    pub keys: Vec<String>,
    pub payload_len: usize,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineMediaRuntimeStats {
    pub transport_state: XbxEngineTransportStateDto,
    pub session_target_type: Option<XbxEngineTargetTypeDto>,
    pub session_phase: Option<String>,
    pub message_handshake_acked_at_ms: Option<f64>,
    pub control_ready_at_ms: Option<f64>,
    pub transport_policy_profile: Option<String>,
    pub recovery_policy_profile: Option<String>,
    pub recovery_diagnosis: Option<String>,
    pub recovery_coupling_mode: Option<String>,
    pub recovery_coupling_summary: Option<String>,
    pub direct_gaming_bitrate_band: Option<String>,
    pub latest_video_frame: Option<XbxEngineVideoFrameStats>,
    pub latest_observation_label: Option<String>,
    pub latest_observation_summary: Option<String>,
    pub latest_video_stream_width: Option<u32>,
    pub latest_video_stream_height: Option<u32>,
    pub first_video_packet_arrival_time_ms: Option<f64>,
    pub latest_video_packet_arrival_time_ms: Option<f64>,
    pub first_audio_packet_arrival_time_ms: Option<f64>,
    pub latest_audio_packet_arrival_time_ms: Option<f64>,
    pub inbound_video_frame_rate_fps: f64,
    pub latest_video_packet_sequence: Option<u16>,
    pub latest_video_packet_gap: Option<XbxEngineVideoPacketGapObservation>,
    pub inbound_video_packet_count_total: u64,
    pub inbound_video_packet_loss_estimate_total: u64,
    pub inbound_video_loss_ratio_1s: f64,
    pub inbound_video_loss_ratio_5s: f64,
    pub inbound_video_jitter_ms: Option<f64>,
    pub video_nack_request_count_total: u64,
    pub video_nack_batch_count_total: u64,
    pub video_nack_per_sec: f64,
    pub latest_video_nack_observation: Option<XbxEngineVideoNackObservation>,
    pub latest_video_escalation_observation: Option<XbxEngineVideoEscalationObservation>,
    pub latest_video_bwe_observation: Option<XbxEngineVideoBweObservation>,
    pub latest_video_twcc_observation: Option<XbxEngineVideoTwccObservation>,
    pub latest_video_track_status: Option<XbxEngineVideoTrackStatus>,
    pub latest_video_repair_probe_observation: Option<XbxEngineVideoRepairProbeObservation>,
    pub latest_video_rtx_reinject_observation: Option<XbxEngineVideoRtxReinjectObservation>,
    pub latest_data_channel_message_catalog_observation:
        Option<XbxEngineDataChannelMessageCatalogObservation>,
    pub transport_recovery_epoch: u64,
    pub transport_recovery_epoch_at_last_escalation: u64,
    pub video_repair_probe_stream_bind_count_total: u64,
    pub video_repair_probe_packet_count_total: u64,
    pub video_repair_probe_active_since_ms: Option<f64>,
    pub video_repair_probe_recovered_count_since_active: u64,
    pub video_repair_probe_late_recovered_count_since_active: u64,
    pub video_repair_probe_expired_count_since_active: u64,
    pub video_repair_probe_packet_gap_count_since_active: u64,
    pub video_repair_probe_recovery_hit_rate_since_active: Option<f64>,
    pub video_rtx_reinject_head_match_count_total: u64,
    pub video_rtx_reinject_range_match_count_total: u64,
    pub video_rtx_reinject_miss_count_total: u64,
    pub video_pli_request_count_total: u64,
    pub video_pli_per_min: f64,
    pub video_pending_missing_packets: usize,
    pub video_loss_finalized_count_total: u64,
    pub video_loss_recovered_count_total: u64,
    pub video_loss_late_recovered_count_total: u64,
    pub video_nack_recovery_rtt_ms: Option<f64>,
    pub video_rtt_ms: Option<f64>,
    pub video_rtt_source: Option<String>,
    pub video_remb_bps: Option<u32>,
    pub inbound_bitrate_kbps: Option<f64>,
    pub inbound_video_bitrate_kbps: Option<f64>,
    pub inbound_audio_bitrate_kbps: Option<f64>,
    pub transport_path: Option<String>,
    pub latest_video_decode_ok_time_ms: Option<f64>,
    pub video_decode_fps: f64,
    pub video_decoder_stalled: Option<bool>,
    pub video_decoder_backend_name: Option<String>,
    pub video_decoder_hardware_failure_streak: u32,
    pub latest_video_decoder_hardware_failure_time_ms: Option<f64>,
    pub latest_video_decoder_hardware_failure_status: Option<i32>,
    pub video_decoder_reset_count: u64,
    pub latest_video_decoder_reset_time_ms: Option<f64>,
    pub video_decode_input_drop_count_total: u64,
    pub video_decode_output_drop_count_total: u64,
    pub video_pacer_submit_count_total: u64,
    pub video_pacer_drop_count_total: u64,
    pub video_renderer_submit_count_total: u64,
    pub video_renderer_drop_count_total: u64,
    pub video_present_overwrite_count_total: u64,
    pub video_present_submit_count_total: u64,
    pub host_display_interval_ms: Option<f64>,
    pub host_frame_age_budget_ms: Option<f64>,
    pub latest_video_present_time_ms: Option<f64>,
    pub video_present_fps: f64,
    pub video_renderer_stalled: Option<bool>,
    pub latest_video_frame_drop: Option<XbxEngineVideoFrameDropObservation>,
    pub inbound_bytes_total: u64,
    pub inbound_video_bytes_total: u64,
    pub inbound_primary_video_bytes_total: u64,
    pub inbound_audio_bytes_total: u64,
}

impl Default for XbxEngineMediaRuntimeStats {
    fn default() -> Self {
        Self {
            transport_state: XbxEngineTransportStateDto::New,
            session_target_type: None,
            session_phase: None,
            message_handshake_acked_at_ms: None,
            control_ready_at_ms: None,
            transport_policy_profile: None,
            recovery_policy_profile: None,
            recovery_diagnosis: None,
            recovery_coupling_mode: None,
            recovery_coupling_summary: None,
            direct_gaming_bitrate_band: None,
            latest_video_frame: None,
            latest_observation_label: None,
            latest_observation_summary: None,
            latest_video_stream_width: None,
            latest_video_stream_height: None,
            first_video_packet_arrival_time_ms: None,
            latest_video_packet_arrival_time_ms: None,
            first_audio_packet_arrival_time_ms: None,
            latest_audio_packet_arrival_time_ms: None,
            inbound_video_frame_rate_fps: 0.0,
            latest_video_packet_sequence: None,
            latest_video_packet_gap: None,
            inbound_video_packet_count_total: 0,
            inbound_video_packet_loss_estimate_total: 0,
            inbound_video_loss_ratio_1s: 0.0,
            inbound_video_loss_ratio_5s: 0.0,
            inbound_video_jitter_ms: None,
            video_nack_request_count_total: 0,
            video_nack_batch_count_total: 0,
            video_nack_per_sec: 0.0,
            latest_video_nack_observation: None,
            latest_video_escalation_observation: None,
            latest_video_bwe_observation: None,
            latest_video_twcc_observation: None,
            latest_video_track_status: None,
            latest_video_repair_probe_observation: None,
            latest_video_rtx_reinject_observation: None,
            latest_data_channel_message_catalog_observation: None,
            transport_recovery_epoch: 0,
            transport_recovery_epoch_at_last_escalation: 0,
            video_repair_probe_stream_bind_count_total: 0,
            video_repair_probe_packet_count_total: 0,
            video_repair_probe_active_since_ms: None,
            video_repair_probe_recovered_count_since_active: 0,
            video_repair_probe_late_recovered_count_since_active: 0,
            video_repair_probe_expired_count_since_active: 0,
            video_repair_probe_packet_gap_count_since_active: 0,
            video_repair_probe_recovery_hit_rate_since_active: None,
            video_rtx_reinject_head_match_count_total: 0,
            video_rtx_reinject_range_match_count_total: 0,
            video_rtx_reinject_miss_count_total: 0,
            video_pli_request_count_total: 0,
            video_pli_per_min: 0.0,
            video_pending_missing_packets: 0,
            video_loss_finalized_count_total: 0,
            video_loss_recovered_count_total: 0,
            video_loss_late_recovered_count_total: 0,
            video_nack_recovery_rtt_ms: None,
            video_rtt_ms: None,
            video_rtt_source: None,
            video_remb_bps: None,
            inbound_bitrate_kbps: None,
            inbound_video_bitrate_kbps: None,
            inbound_audio_bitrate_kbps: None,
            transport_path: None,
            latest_video_decode_ok_time_ms: None,
            video_decode_fps: 0.0,
            video_decoder_stalled: None,
            video_decoder_backend_name: None,
            video_decoder_hardware_failure_streak: 0,
            latest_video_decoder_hardware_failure_time_ms: None,
            latest_video_decoder_hardware_failure_status: None,
            video_decoder_reset_count: 0,
            latest_video_decoder_reset_time_ms: None,
            video_decode_input_drop_count_total: 0,
            video_decode_output_drop_count_total: 0,
            video_pacer_submit_count_total: 0,
            video_pacer_drop_count_total: 0,
            video_renderer_submit_count_total: 0,
            video_renderer_drop_count_total: 0,
            video_present_overwrite_count_total: 0,
            video_present_submit_count_total: 0,
            host_display_interval_ms: None,
            host_frame_age_budget_ms: None,
            latest_video_present_time_ms: None,
            video_present_fps: 0.0,
            video_renderer_stalled: None,
            latest_video_frame_drop: None,
            inbound_bytes_total: 0,
            inbound_video_bytes_total: 0,
            inbound_primary_video_bytes_total: 0,
            inbound_audio_bytes_total: 0,
        }
    }
}

pub trait XbxEngineMediaBackend: Send {
    fn sync_runtime_config(
        &mut self,
        _runtime_config: &XbxEngineRuntimeConfig,
    ) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }

    fn negotiate(
        &mut self,
        request: XbxEngineMediaNegotiationRequest,
    ) -> Result<XbxEngineMediaNegotiation, XbxEngineRuntimeError>;
    fn create_offer(&mut self) -> Result<String, XbxEngineRuntimeError>;
    fn apply_remote_description(
        &mut self,
        answer_sdp: String,
        remote_candidates: Vec<XbxEngineIceCandidateDto>,
    ) -> Result<(), XbxEngineRuntimeError>;
    fn add_remote_ice_candidates(
        &mut self,
        remote_candidates: Vec<XbxEngineIceCandidateDto>,
    ) -> Result<(), XbxEngineRuntimeError>;
    fn local_candidates_snapshot(
        &self,
    ) -> Result<Vec<XbxEngineIceCandidateDto>, XbxEngineRuntimeError>;
    fn local_ice_gathering_complete(&self) -> Result<bool, XbxEngineRuntimeError>;
    fn apply_display_state(
        &mut self,
        state: XbxEngineDisplayStateDto,
    ) -> Result<(), XbxEngineRuntimeError>;
    fn set_audio_volume(&mut self, value: f32) -> Result<(), XbxEngineRuntimeError>;
    fn set_microphone_capturing(&mut self, capturing: bool) -> Result<(), XbxEngineRuntimeError>;
    fn press_controller_button(
        &mut self,
        button: String,
        duration_ms: u64,
    ) -> Result<(), XbxEngineRuntimeError>;
    fn set_keyboard_pointer_enabled(&mut self, enabled: bool) -> Result<(), XbxEngineRuntimeError>;
    fn push_keyboard_pointer_input(
        &mut self,
        event: XbxEngineInputEventDto,
    ) -> Result<(), XbxEngineRuntimeError>;
    fn current_input_status(&self) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError>;
    fn snapshot_runtime_stats(&self) -> Result<XbxEngineMediaRuntimeStats, XbxEngineRuntimeError>;
    fn take_pending_gamepad_rumble_requests(
        &mut self,
    ) -> Result<Vec<OhMyGamepadRumbleRequestDto>, XbxEngineRuntimeError> {
        Ok(Vec::new())
    }
    fn take_pending_runtime_recovery_action(
        &mut self,
    ) -> Result<Option<XbxEnginePendingRuntimeRecoveryAction>, XbxEngineRuntimeError> {
        Ok(None)
    }
    fn update_host_video_timing(
        &mut self,
        _host_display_interval_ms: Option<f64>,
        _host_frame_age_budget_ms: Option<f64>,
    ) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }
    fn take_latest_render_frame(
        &mut self,
    ) -> Result<Option<XbxEngineRenderFrame>, XbxEngineRuntimeError>;
    fn request_video_keyframe(&mut self) -> Result<(), XbxEngineRuntimeError>;
    fn request_decoder_reset(&mut self) -> Result<(), XbxEngineRuntimeError>;
    fn stop(&mut self) -> Result<(), XbxEngineRuntimeError>;
}

impl<TMediaBackend> XbxEngineMediaBackend for Box<TMediaBackend>
where
    TMediaBackend: XbxEngineMediaBackend + ?Sized,
{
    fn sync_runtime_config(
        &mut self,
        runtime_config: &XbxEngineRuntimeConfig,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut().sync_runtime_config(runtime_config)
    }

    fn negotiate(
        &mut self,
        request: XbxEngineMediaNegotiationRequest,
    ) -> Result<XbxEngineMediaNegotiation, XbxEngineRuntimeError> {
        self.as_mut().negotiate(request)
    }

    fn create_offer(&mut self) -> Result<String, XbxEngineRuntimeError> {
        self.as_mut().create_offer()
    }

    fn apply_remote_description(
        &mut self,
        answer_sdp: String,
        remote_candidates: Vec<XbxEngineIceCandidateDto>,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut()
            .apply_remote_description(answer_sdp, remote_candidates)
    }

    fn add_remote_ice_candidates(
        &mut self,
        remote_candidates: Vec<XbxEngineIceCandidateDto>,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut().add_remote_ice_candidates(remote_candidates)
    }

    fn local_candidates_snapshot(
        &self,
    ) -> Result<Vec<XbxEngineIceCandidateDto>, XbxEngineRuntimeError> {
        self.as_ref().local_candidates_snapshot()
    }

    fn local_ice_gathering_complete(&self) -> Result<bool, XbxEngineRuntimeError> {
        self.as_ref().local_ice_gathering_complete()
    }

    fn apply_display_state(
        &mut self,
        state: XbxEngineDisplayStateDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut().apply_display_state(state)
    }

    fn set_audio_volume(&mut self, value: f32) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut().set_audio_volume(value)
    }

    fn set_microphone_capturing(&mut self, capturing: bool) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut().set_microphone_capturing(capturing)
    }

    fn press_controller_button(
        &mut self,
        button: String,
        duration_ms: u64,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut().press_controller_button(button, duration_ms)
    }

    fn set_keyboard_pointer_enabled(&mut self, enabled: bool) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut().set_keyboard_pointer_enabled(enabled)
    }

    fn push_keyboard_pointer_input(
        &mut self,
        event: XbxEngineInputEventDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut().push_keyboard_pointer_input(event)
    }

    fn current_input_status(&self) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError> {
        self.as_ref().current_input_status()
    }

    fn snapshot_runtime_stats(&self) -> Result<XbxEngineMediaRuntimeStats, XbxEngineRuntimeError> {
        self.as_ref().snapshot_runtime_stats()
    }

    fn take_pending_gamepad_rumble_requests(
        &mut self,
    ) -> Result<Vec<OhMyGamepadRumbleRequestDto>, XbxEngineRuntimeError> {
        self.as_mut().take_pending_gamepad_rumble_requests()
    }

    fn take_pending_runtime_recovery_action(
        &mut self,
    ) -> Result<Option<XbxEnginePendingRuntimeRecoveryAction>, XbxEngineRuntimeError> {
        self.as_mut().take_pending_runtime_recovery_action()
    }

    fn update_host_video_timing(
        &mut self,
        host_display_interval_ms: Option<f64>,
        host_frame_age_budget_ms: Option<f64>,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut()
            .update_host_video_timing(host_display_interval_ms, host_frame_age_budget_ms)
    }

    fn take_latest_render_frame(
        &mut self,
    ) -> Result<Option<XbxEngineRenderFrame>, XbxEngineRuntimeError> {
        self.as_mut().take_latest_render_frame()
    }

    fn request_video_keyframe(&mut self) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut().request_video_keyframe()
    }

    fn request_decoder_reset(&mut self) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut().request_decoder_reset()
    }

    fn stop(&mut self) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut().stop()
    }
}

pub struct PlaceholderXbxEngineMediaBackend {
    input_backend: Box<dyn XbxEngineInputBackend>,
    pub negotiation_count: usize,
    pub last_offer_sdp: Option<String>,
    pub last_answer_sdp: Option<String>,
    pub last_remote_candidates: Vec<XbxEngineIceCandidateDto>,
    pub last_display_state: Option<XbxEngineDisplayStateDto>,
    pub audio_volume: f32,
    pub microphone_capturing: bool,
    pub keyboard_pointer_enabled: bool,
    pub last_keyboard_pointer_event: Option<XbxEngineInputEventDto>,
    pub last_pressed_controller_button: Option<(String, u64)>,
    pub last_input_status: XbxEngineInputStatus,
    pub last_runtime_stats: XbxEngineMediaRuntimeStats,
    pub pending_runtime_recovery_action: Option<XbxEnginePendingRuntimeRecoveryAction>,
}

impl Default for PlaceholderXbxEngineMediaBackend {
    fn default() -> Self {
        Self::with_input_backend(Box::<NoopXbxEngineInputBackend>::default())
    }
}

impl PlaceholderXbxEngineMediaBackend {
    pub fn with_input_backend(input_backend: Box<dyn XbxEngineInputBackend>) -> Self {
        Self {
            input_backend,
            negotiation_count: 0,
            last_offer_sdp: None,
            last_answer_sdp: None,
            last_remote_candidates: Vec::new(),
            last_display_state: None,
            audio_volume: 1.0,
            microphone_capturing: false,
            keyboard_pointer_enabled: false,
            last_keyboard_pointer_event: None,
            last_pressed_controller_button: None,
            last_input_status: XbxEngineInputStatus::default(),
            last_runtime_stats: XbxEngineMediaRuntimeStats::default(),
            pending_runtime_recovery_action: None,
        }
    }
}

impl XbxEngineMediaBackend for PlaceholderXbxEngineMediaBackend {
    fn sync_runtime_config(
        &mut self,
        _runtime_config: &XbxEngineRuntimeConfig,
    ) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }

    fn negotiate(
        &mut self,
        request: XbxEngineMediaNegotiationRequest,
    ) -> Result<XbxEngineMediaNegotiation, XbxEngineRuntimeError> {
        self.negotiation_count += 1;
        self.last_input_status = self
            .input_backend
            .attach_session(&request.session.session_id)?;

        let offer_sdp = if request.restart {
            format!(
                "v=0\r\no={} restart-placeholder:{}\r\n",
                request.session.session_id, self.negotiation_count
            )
        } else {
            format!(
                "v=0\r\no={} initial-placeholder:{}\r\n",
                request.session.session_id, self.negotiation_count
            )
        };
        self.last_offer_sdp = Some(offer_sdp.clone());

        let frame_clock = now_ms_f64();
        self.last_runtime_stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_frame: Some(XbxEngineVideoFrameStats {
                width: 1280,
                height: 720,
                frame_seq: self.negotiation_count as u64,
                fps: 60.0,
                rendered_at_ms: frame_clock + 12.0,
            }),
            ..Default::default()
        };
        // 让默认测试后端也具备一条最小可用的 ICE 候选，避免 runtime 依赖空交换兜底。
        let local_candidate = XbxEngineIceCandidateDto {
            candidate: "candidate:placeholder 1 udp 2130706431 127.0.0.1 60000 typ host"
                .to_string(),
            sdp_m_line_index: Some(0),
            sdp_mid: Some("0".to_string()),
        };
        Ok(XbxEngineMediaNegotiation {
            local_offer_sdp: offer_sdp,
            local_candidates: vec![local_candidate],
            surface_id: format!("surface:{}", request.viewport.viewport_id),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: Some(frame_clock),
            frame_decoded_time_ms: Some(frame_clock + 8.0),
            frame_rendered_time_ms: Some(frame_clock + 12.0),
            input_status: self.last_input_status.clone(),
        })
    }

    fn apply_remote_description(
        &mut self,
        answer_sdp: String,
        remote_candidates: Vec<XbxEngineIceCandidateDto>,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.last_answer_sdp = Some(answer_sdp);
        self.last_remote_candidates = remote_candidates;
        Ok(())
    }

    fn add_remote_ice_candidates(
        &mut self,
        remote_candidates: Vec<XbxEngineIceCandidateDto>,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.last_remote_candidates.extend(remote_candidates);
        Ok(())
    }

    fn local_candidates_snapshot(
        &self,
    ) -> Result<Vec<XbxEngineIceCandidateDto>, XbxEngineRuntimeError> {
        Ok(Vec::new())
    }

    fn local_ice_gathering_complete(&self) -> Result<bool, XbxEngineRuntimeError> {
        Ok(true)
    }

    fn create_offer(&mut self) -> Result<String, XbxEngineRuntimeError> {
        let next_offer = format!(
            "v=0\r\no=placeholder chat-offer:{}\r\n",
            self.negotiation_count.saturating_add(1)
        );
        self.last_offer_sdp = Some(next_offer.clone());
        Ok(next_offer)
    }

    fn apply_display_state(
        &mut self,
        state: XbxEngineDisplayStateDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.last_display_state = Some(state);
        Ok(())
    }

    fn set_audio_volume(&mut self, value: f32) -> Result<(), XbxEngineRuntimeError> {
        self.audio_volume = value;
        Ok(())
    }

    fn set_microphone_capturing(&mut self, capturing: bool) -> Result<(), XbxEngineRuntimeError> {
        self.microphone_capturing = capturing;
        Ok(())
    }

    fn press_controller_button(
        &mut self,
        button: String,
        duration_ms: u64,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.last_pressed_controller_button = Some((button, duration_ms));
        Ok(())
    }

    fn set_keyboard_pointer_enabled(&mut self, enabled: bool) -> Result<(), XbxEngineRuntimeError> {
        self.keyboard_pointer_enabled = enabled;
        Ok(())
    }

    fn push_keyboard_pointer_input(
        &mut self,
        event: XbxEngineInputEventDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.last_keyboard_pointer_event = Some(event);
        Ok(())
    }

    fn current_input_status(&self) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError> {
        Ok(self.last_input_status.clone())
    }

    fn snapshot_runtime_stats(&self) -> Result<XbxEngineMediaRuntimeStats, XbxEngineRuntimeError> {
        Ok(self.last_runtime_stats.clone())
    }

    fn take_pending_runtime_recovery_action(
        &mut self,
    ) -> Result<Option<XbxEnginePendingRuntimeRecoveryAction>, XbxEngineRuntimeError> {
        Ok(self.pending_runtime_recovery_action.take())
    }

    fn take_latest_render_frame(
        &mut self,
    ) -> Result<Option<XbxEngineRenderFrame>, XbxEngineRuntimeError> {
        Ok(None)
    }

    fn request_video_keyframe(&mut self) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }

    fn request_decoder_reset(&mut self) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }

    fn stop(&mut self) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }
}

fn now_ms_f64() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}
