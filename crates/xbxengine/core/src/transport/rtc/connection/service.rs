use std::sync::{Arc, Mutex};

use rtc::rtcp::payload_feedbacks::receiver_estimated_maximum_bitrate::ReceiverEstimatedMaximumBitrate;

use crate::api::runtime::XbxEngineWebRtcRuntimeConfig;
use crate::runtime_stats_sink::RuntimeStatsSink;
#[allow(unused_imports)]
pub(super) use crate::transport::rtc::connection::builder::{
    build_owned_h264_codec_preferences, register_owned_h264_codecs, ControlledPeerConnection,
};
use crate::transport::rtc::connection::control_channel::RtcControlChannelService;
use crate::transport::rtc::connection::io_runtime::RtcIoRuntime;
use crate::transport::rtc::connection::runtime_state::RtcConnectionRuntimeState;
use crate::transport::rtc::connection::transport_metrics::describe_selected_candidate_pair;
use crate::transport::rtc::connection::twcc_feedback::ControlledTwccFeedbackController;
use crate::transport::rtc::connection::{
    build_control_decoder_reset_payload, build_control_keyframe_request_payload,
};
use crate::transport::rtc::events::RtcConnectionLifecycleState;
use crate::transport::rtc::facts::TransportFact;
use crate::transport::rtc::stream::{RtcMediaIngressPacket, RtcRtpPacketMeta};
use crate::{XbxEngineMediaRuntimeStats, XbxEngineRuntimeError};

pub(crate) struct RtcConnectionService {
    pub(super) state: Arc<Mutex<RtcConnectionRuntimeState>>,
    pub(super) peer_connection: Option<ControlledPeerConnection>,
    pub(super) io_runtime: RtcIoRuntime,
    pub(super) control_service: RtcControlChannelService,
    pub(super) webrtc_runtime_config: XbxEngineWebRtcRuntimeConfig,
    pub(super) lifecycle_state: RtcConnectionLifecycleState,
    pub(super) lifecycle_state_since_ms: f64,
    pub(super) last_transport_metrics_sample_at_ms: f64,
    pub(super) last_transport_metrics_sample_inbound_video_bytes_total: u64,
    pub(super) lifecycle_observation_id: u64,
    pub(super) remote_rtcp_twcc_observation_id: u64,
    pub(super) controlled_twcc_feedback: ControlledTwccFeedbackController,
    pub(super) pump_failure_injected: bool,
    pub(super) read_counters: RtcReadIngressCounters,
    pub(super) pending_media_ingress_packets:
        Vec<(RtcMediaIngressPacket, Option<RtcRtpPacketMeta>)>,
    pub(super) pending_gamepad_rumble_requests:
        Vec<ohmygamepad_protocol::OhMyGamepadRumbleRequestDto>,
    pub(super) pending_transport_facts: Vec<TransportFact>,
    pub(super) delayed_gamepad_added_due_at_ms: Option<f64>,
    pub(super) delayed_keyframe_prime_due_at_ms: Option<f64>,
    pub(super) pending_target_remb_kbps: Option<u32>,
    pub(super) active_target_remb_kbps: Option<u32>,
    pub(super) last_target_remb_request_at_ms: Option<f64>,
    pub(super) last_target_remb_requested_kbps: Option<u32>,
    pub(super) target_remb_request_count: u64,
    pub(super) last_selected_pair_diagnostic: Option<String>,
    pub(super) selected_pair_snapshot_emitted: bool,
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
            remote_rtcp_twcc_observation_id: 0,
            controlled_twcc_feedback: ControlledTwccFeedbackController::new(
                webrtc_runtime_config.video_pipeline.feedback_interval_ms,
            ),
            pump_failure_injected: false,
            read_counters: RtcReadIngressCounters::default(),
            pending_media_ingress_packets: Vec::new(),
            pending_gamepad_rumble_requests: Vec::new(),
            pending_transport_facts: Vec::new(),
            delayed_gamepad_added_due_at_ms: None,
            delayed_keyframe_prime_due_at_ms: None,
            pending_target_remb_kbps: None,
            active_target_remb_kbps: None,
            last_target_remb_request_at_ms: None,
            last_target_remb_requested_kbps: None,
            target_remb_request_count: 0,
            last_selected_pair_diagnostic: None,
            selected_pair_snapshot_emitted: false,
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
        self.controlled_twcc_feedback
            .set_feedback_interval(runtime_config.video_pipeline.feedback_interval_ms);
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

    pub(crate) fn request_target_remb_kbps(
        &mut self,
        target_kbps: u32,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let Some(peer_connection) = self.peer_connection.as_mut() else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineRtcPeerConnectionMissingForTargetRemb",
            ));
        };
        let Some((receiver_id, media_ssrc)) = self
            .controlled_twcc_feedback
            .preferred_video_feedback_target()
        else {
            self.pending_target_remb_kbps = Some(target_kbps);
            RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
                stats.latest_observation_label = Some("rtcTargetRembQueued".to_string());
                stats.latest_observation_summary =
                    Some(format!("phase1 rtc target remb queued {}kbps", target_kbps));
                stats.latest_target_remb_action = Some("queued".to_string());
                stats.latest_target_remb_summary =
                    Some(format!("phase1 rtc target remb queued {}kbps", target_kbps));
            });
            return Ok(());
        };
        let Some(mut receiver) = peer_connection.rtp_receiver(receiver_id) else {
            self.pending_target_remb_kbps = Some(target_kbps);
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineRtcReceiverLookupFailedForTargetRemb",
            ));
        };

        let remb = ReceiverEstimatedMaximumBitrate {
            sender_ssrc: 0,
            bitrate: (target_kbps as f32) * 1_000.0,
            ssrcs: media_ssrc.into_iter().collect(),
        };
        receiver.write_rtcp(vec![Box::new(remb)]).map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcWriteTargetRembFailed: {err}"))
        })?;

        self.target_remb_request_count = self.target_remb_request_count.saturating_add(1);
        self.active_target_remb_kbps = Some(target_kbps);
        self.last_target_remb_request_at_ms = Some(crate::transport::rtc::stats::now_ms_f64());
        self.last_target_remb_requested_kbps = Some(target_kbps);
        RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
            stats.latest_observation_label = Some("rtcTargetRembRequested".to_string());
            stats.latest_observation_summary = Some(format!(
                "phase1 rtc target remb requested {}kbps mediaSsrc={media_ssrc:?}",
                target_kbps
            ));
            stats.latest_target_remb_action = Some("requested".to_string());
            stats.latest_target_remb_summary = Some(format!(
                "phase1 rtc target remb requested {}kbps mediaSsrc={media_ssrc:?} count={}",
                target_kbps, self.target_remb_request_count,
            ));
        });
        self.pending_target_remb_kbps = None;

        Ok(())
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
            let pair_diagnostic = describe_selected_candidate_pair(peer_connection);
            if !self.selected_pair_snapshot_emitted
                || pair_diagnostic != self.last_selected_pair_diagnostic
            {
                crate::xbx_log_warn!(
                    "[xbxengine][rtc-connection] selected_pair_snapshot {}",
                    pair_diagnostic.as_deref().unwrap_or("none")
                );
                self.last_selected_pair_diagnostic = pair_diagnostic;
                self.selected_pair_snapshot_emitted = true;
            }
        }
        crate::xbx_log_warn!("[xbxengine][rtc-connection] pump after io_runtime");
        self.drain_peer_events(runtime_stats)?;
        self.drain_peer_reads_core(runtime_stats)?;
        if let Some(target_kbps) = self.pending_target_remb_kbps {
            let _ = self.request_target_remb_kbps(target_kbps, runtime_stats);
        } else if let Some(target_kbps) =
            desired_target_remb_kbps(runtime_stats).or(self.active_target_remb_kbps)
        {
            if self.should_refresh_target_remb(target_kbps, runtime_stats) {
                let _ = self.request_target_remb_kbps(target_kbps, runtime_stats);
            }
        }
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

fn desired_target_remb_kbps(runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>) -> Option<u32> {
    RuntimeStatsSink::read_shared(runtime_stats, |stats| {
        stats
            .latest_video_bwe_observation
            .as_ref()
            .map(|bwe| bwe.target_remb_kbps)
            .or_else(|| stats.video_remb_bps.map(|bps| bps / 1_000))
    })
    .flatten()
}

impl RtcConnectionService {
    fn should_refresh_target_remb(
        &self,
        target_kbps: u32,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> bool {
        if self.last_target_remb_requested_kbps != Some(target_kbps) {
            return true;
        }
        let base_interval_ms = self
            .webrtc_runtime_config
            .video_pipeline
            .feedback_interval_ms
            .max(100);
        let pressure_interval_ms = (base_interval_ms / 2).clamp(100, base_interval_ms);
        let refresh_interval_ms = RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            let recent_gap = stats
                .latest_video_packet_gap
                .as_ref()
                .is_some_and(|gap| (now_ms - gap.observed_at_ms).max(0.0) <= 250.0);
            let recent_nack = stats
                .latest_video_nack_observation
                .as_ref()
                .is_some_and(|nack| (now_ms - nack.observed_at_ms).max(0.0) <= 350.0);
            if stats.video_pending_missing_packets > 0
                || stats.recovery_coupling_mode.as_deref() == Some("waitingKeyframe")
                || recent_gap
                || recent_nack
            {
                pressure_interval_ms
            } else {
                base_interval_ms
            }
        })
        .unwrap_or(base_interval_ms);
        let Some(last_request_at_ms) = self.last_target_remb_request_at_ms else {
            return true;
        };
        (crate::transport::rtc::stats::now_ms_f64() - last_request_at_ms).max(0.0)
            >= refresh_interval_ms as f64
    }
}

#[cfg(test)]
#[path = "tests/service.rs"]
mod tests;
