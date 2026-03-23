use std::sync::{Arc, Mutex};

use rtc::peer_connection::RTCPeerConnection;

use crate::api::runtime::XbxEngineWebRtcRuntimeConfig;
#[allow(unused_imports)]
pub(super) use crate::transport::rtc::connection::builder::{
    build_owned_h264_codec_preferences, register_owned_h264_codecs,
};
use crate::transport::rtc::connection::control_channel::RtcControlChannelService;
use crate::transport::rtc::connection::io_runtime::RtcIoRuntime;
use crate::transport::rtc::connection::runtime_state::RtcConnectionRuntimeState;
use crate::transport::rtc::connection::{
    build_control_decoder_reset_payload, build_control_keyframe_request_payload,
};
use crate::transport::rtc::events::RtcConnectionLifecycleState;
use crate::transport::rtc::facts::TransportFact;
use crate::transport::rtc::stream::{RtcMediaIngressPacket, RtcRtpPacketMeta};
use crate::{XbxEngineMediaRuntimeStats, XbxEngineRuntimeError};

pub(crate) struct RtcConnectionService {
    pub(super) state: Arc<Mutex<RtcConnectionRuntimeState>>,
    pub(super) peer_connection: Option<RTCPeerConnection>,
    pub(super) io_runtime: RtcIoRuntime,
    pub(super) control_service: RtcControlChannelService,
    pub(super) webrtc_runtime_config: XbxEngineWebRtcRuntimeConfig,
    pub(super) lifecycle_state: RtcConnectionLifecycleState,
    pub(super) lifecycle_state_since_ms: f64,
    pub(super) last_transport_metrics_sample_at_ms: f64,
    pub(super) last_transport_metrics_sample_inbound_video_bytes_total: u64,
    pub(super) lifecycle_observation_id: u64,
    pub(super) twcc_observation_id: u64,
    pub(super) pump_failure_injected: bool,
    pub(super) read_counters: RtcReadIngressCounters,
    pub(super) pending_media_ingress_packets:
        Vec<(RtcMediaIngressPacket, Option<RtcRtpPacketMeta>)>,
    pub(super) pending_gamepad_rumble_requests:
        Vec<ohmygamepad_protocol::OhMyGamepadRumbleRequestDto>,
    pub(super) pending_transport_facts: Vec<TransportFact>,
    pub(super) delayed_gamepad_added_due_at_ms: Option<f64>,
    pub(super) delayed_keyframe_prime_due_at_ms: Option<f64>,
}

impl Default for RtcConnectionService {
    fn default() -> Self {
        let webrtc_runtime_config = XbxEngineWebRtcRuntimeConfig::default();
        Self {
            state: Arc::new(Mutex::new(RtcConnectionRuntimeState::default())),
            peer_connection: None,
            io_runtime: RtcIoRuntime::default(),
            control_service: RtcControlChannelService::default(),
            webrtc_runtime_config: webrtc_runtime_config.clone(),
            lifecycle_state: RtcConnectionLifecycleState::Closed,
            lifecycle_state_since_ms: 0.0,
            last_transport_metrics_sample_at_ms: 0.0,
            last_transport_metrics_sample_inbound_video_bytes_total: 0,
            lifecycle_observation_id: 0,
            twcc_observation_id: 0,
            pump_failure_injected: false,
            read_counters: RtcReadIngressCounters::default(),
            pending_media_ingress_packets: Vec::new(),
            pending_gamepad_rumble_requests: Vec::new(),
            pending_transport_facts: Vec::new(),
            delayed_gamepad_added_due_at_ms: None,
            delayed_keyframe_prime_due_at_ms: None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct RtcReadIngressCounters {
    pub(super) rtp_packets: u64,
    pub(super) rtcp_packets: u64,
    pub(super) data_channel_messages: u64,
    pub(super) last_data_channel_label: Option<String>,
}

pub(super) const RTC_RECONNECT_GRACE_MS: f64 = 750.0;
impl RtcConnectionService {
    pub(crate) fn sync_runtime_config(&mut self, runtime_config: XbxEngineWebRtcRuntimeConfig) {
        self.webrtc_runtime_config = runtime_config;
    }

    pub(crate) fn set_keyboard_pointer_enabled(&mut self, enabled: bool) {
        self.control_service.set_keyboard_pointer_enabled(enabled);
    }

    pub(crate) fn request_video_keyframe(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        if let Err(error) = self.control_service.request_video_keyframe() {
            return Err(error);
        }
        self.send_control_payload(
            build_control_keyframe_request_payload(),
            "rtcControlKeyframeRequested",
            "phase1 rtc control keyframe requested",
            runtime_stats,
        )
    }

    pub(crate) fn request_decoder_reset(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        if let Err(error) = self.control_service.request_decoder_reset() {
            return Err(error);
        }
        self.send_control_payload(
            build_control_decoder_reset_payload(),
            "rtcControlDecoderResetRequested",
            "phase1 rtc control decoder reset requested",
            runtime_stats,
        )
    }

    pub(crate) fn pump(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        crate::xbx_log_warn!("[xbxengine][rtc-connection] pump enter");
        if self.pump_failure_injected {
            self.pump_failure_injected = false;
            let error = XbxEngineRuntimeError::new("xbxEngineRtcPumpInjectedFailure");
            self.mark_recovering_from_fault(
                runtime_stats,
                "rtcPumpFailed",
                "phase1 rtc injected pump failure",
                RtcConnectionLifecycleState::Failed,
                error.to_string(),
            );
            return Err(error);
        }
        if let Some(peer_connection) = self.peer_connection.as_mut() {
            if let Err(error) = self.io_runtime.pump(peer_connection) {
                self.mark_recovering_from_fault(
                    runtime_stats,
                    "rtcPumpFailed",
                    "phase1 rtc io pump failed",
                    RtcConnectionLifecycleState::Failed,
                    error.to_string(),
                );
                return Err(error);
            }
        }
        crate::xbx_log_warn!("[xbxengine][rtc-connection] pump after io_runtime");
        self.drain_peer_events(runtime_stats)?;
        self.drain_peer_reads_core(runtime_stats)?;
        crate::xbx_log_warn!("[xbxengine][rtc-connection] pump after drain peer events/reads");
        self.try_send_message_handshake(runtime_stats)?;
        self.run_delayed_control_actions(runtime_stats)?;
        self.maybe_schedule_delayed_reconnect(runtime_stats);
        self.refresh_transport_metrics(runtime_stats);
        crate::xbx_log_warn!("[xbxengine][rtc-connection] pump exit");
        Ok(())
    }

    pub(crate) fn take_media_ingress_packets(
        &mut self,
    ) -> Vec<(RtcMediaIngressPacket, Option<RtcRtpPacketMeta>)> {
        std::mem::take(&mut self.pending_media_ingress_packets)
    }

    pub(crate) fn take_pending_gamepad_rumble_requests(
        &mut self,
    ) -> Vec<ohmygamepad_protocol::OhMyGamepadRumbleRequestDto> {
        std::mem::take(&mut self.pending_gamepad_rumble_requests)
    }

    pub(crate) fn take_transport_facts(&mut self) -> Vec<TransportFact> {
        std::mem::take(&mut self.pending_transport_facts)
    }

    #[cfg(test)]
    pub(crate) fn inject_pump_failure(&mut self) {
        self.pump_failure_injected = true;
    }
}

#[cfg(test)]
#[path = "tests/service.rs"]
mod tests;
