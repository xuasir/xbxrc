use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use xbxengine_protocol::{
    XbxEngineDisplayStateDto, XbxEngineIceCandidateDto, XbxEngineInputEventDto,
    XbxEngineSessionDto, XbxEngineTransportStateDto, XbxEngineViewportDto,
};

use crate::api::input::{NoopXbxEngineInputBackend, XbxEngineInputBackend, XbxEngineInputStatus};
use crate::api::runtime::XbxEngineRuntimeError;

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

pub struct MacOsCVPixelBufferDescriptor {
    pub ptr: *mut std::ffi::c_void,
    pub drop_fn: Option<Box<dyn FnOnce(*mut std::ffi::c_void) + Send + Sync>>,
}

impl std::fmt::Debug for MacOsCVPixelBufferDescriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MacOsCVPixelBufferDescriptor")
            .field("ptr", &self.ptr)
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
pub struct XbxEngineMediaRuntimeStats {
    pub transport_state: XbxEngineTransportStateDto,
    pub latest_video_frame: Option<XbxEngineVideoFrameStats>,
    pub latest_video_stream_width: Option<u32>,
    pub latest_video_stream_height: Option<u32>,
    pub latest_video_packet_arrival_time_ms: Option<f64>,
    pub latest_video_packet_sequence: Option<u16>,
    pub inbound_video_packet_count_total: u64,
    pub inbound_video_packet_loss_estimate_total: u64,
    pub inbound_video_loss_ratio_1s: f64,
    pub inbound_video_loss_ratio_5s: f64,
    pub inbound_video_jitter_ms: Option<f64>,
    pub video_nack_request_count_total: u64,
    pub video_nack_batch_count_total: u64,
    pub video_nack_per_sec: f64,
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
    pub latest_video_decode_ok_time_ms: Option<f64>,
    pub video_decoder_stalled: Option<bool>,
    pub video_decoder_backend_name: Option<String>,
    pub video_decoder_reset_count: u64,
    pub latest_video_decoder_reset_time_ms: Option<f64>,
    pub video_decode_input_drop_count_total: u64,
    pub video_decode_output_drop_count_total: u64,
    pub latest_video_present_time_ms: Option<f64>,
    pub video_renderer_stalled: Option<bool>,
    pub inbound_bytes_total: u64,
    pub inbound_video_bytes_total: u64,
    pub inbound_primary_video_bytes_total: u64,
    pub inbound_audio_bytes_total: u64,
}

impl Default for XbxEngineMediaRuntimeStats {
    fn default() -> Self {
        Self {
            transport_state: XbxEngineTransportStateDto::New,
            latest_video_frame: None,
            latest_video_stream_width: None,
            latest_video_stream_height: None,
            latest_video_packet_arrival_time_ms: None,
            latest_video_packet_sequence: None,
            inbound_video_packet_count_total: 0,
            inbound_video_packet_loss_estimate_total: 0,
            inbound_video_loss_ratio_1s: 0.0,
            inbound_video_loss_ratio_5s: 0.0,
            inbound_video_jitter_ms: None,
            video_nack_request_count_total: 0,
            video_nack_batch_count_total: 0,
            video_nack_per_sec: 0.0,
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
            latest_video_decode_ok_time_ms: None,
            video_decoder_stalled: None,
            video_decoder_backend_name: None,
            video_decoder_reset_count: 0,
            latest_video_decoder_reset_time_ms: None,
            video_decode_input_drop_count_total: 0,
            video_decode_output_drop_count_total: 0,
            latest_video_present_time_ms: None,
            video_renderer_stalled: None,
            inbound_bytes_total: 0,
            inbound_video_bytes_total: 0,
            inbound_primary_video_bytes_total: 0,
            inbound_audio_bytes_total: 0,
        }
    }
}

pub trait XbxEngineMediaBackend: Send {
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
        }
    }
}

impl XbxEngineMediaBackend for PlaceholderXbxEngineMediaBackend {
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
        Ok(XbxEngineMediaNegotiation {
            local_offer_sdp: offer_sdp,
            local_candidates: Vec::new(),
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
