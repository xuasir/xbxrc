use rtc::rtcp::payload_feedbacks::full_intra_request::{FirEntry, FullIntraRequest};
use rtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use rtc::rtcp::payload_feedbacks::receiver_estimated_maximum_bitrate::ReceiverEstimatedMaximumBitrate;
use std::sync::{Arc, Mutex};

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

const VIDEO_RECOVERY_PLI_TO_FIR_MIN_DELAY_MS: f64 = 180.0;
const VIDEO_RECOVERY_FIR_TO_CONTROL_MIN_DELAY_MS: f64 = 360.0;
// 与恢复侧的 escalation window 对齐，作为首个 keyframe 响应的统一观测窗口。
const KEYFRAME_REQUEST_RESPONSE_WINDOW_MS: f64 = 960.0;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum VideoRecoveryTransportStage {
    #[default]
    None,
    PictureLossIndication,
    FullIntraRequest,
    ControlKeyframe,
}

#[derive(Clone, Debug, Default)]
pub(super) struct VideoRecoveryTransportState {
    recovery_epoch: u64,
    stage: VideoRecoveryTransportStage,
    last_sent_at_ms: Option<f64>,
}

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
    pub(super) local_rtcp_sender_ssrc: u32,
    pub(super) last_selected_pair_diagnostic: Option<String>,
    pub(super) selected_pair_snapshot_emitted: bool,
    pub(super) video_recovery_transport_state: VideoRecoveryTransportState,
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
            local_rtcp_sender_ssrc: generate_local_rtcp_sender_ssrc(),
            last_selected_pair_diagnostic: None,
            selected_pair_snapshot_emitted: false,
            video_recovery_transport_state: VideoRecoveryTransportState::default(),
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

impl RtcConnectionService {
    pub(crate) fn sync_runtime_config(&mut self, runtime_config: XbxEngineWebRtcRuntimeConfig) {
        self.controlled_twcc_feedback
            .set_feedback_interval(runtime_config.video_pipeline.feedback_interval_ms);
        self.io_runtime
            .set_prefer_ipv6(runtime_config.negotiation.prefer_ipv6);
        self.webrtc_runtime_config = runtime_config;
    }

    pub(crate) fn set_keyboard_pointer_enabled(&mut self, enabled: bool) {
        self.control_service.set_keyboard_pointer_enabled(enabled);
    }

    pub(crate) fn request_video_keyframe(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.request_video_recovery_keyframe(runtime_stats)
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

    fn request_video_recovery_keyframe(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.sync_video_recovery_transport_state(runtime_stats);
        match self.resolve_video_recovery_transport_stage(runtime_stats) {
            VideoRecoveryTransportStage::None => {
                self.record_video_recovery_observation(
                    runtime_stats,
                    "rtcVideoRecoverySuppressed",
                    "phase1 rtc video recovery suppressed by active clean anchor or duplicate stage",
                );
                Ok(())
            }
            VideoRecoveryTransportStage::PictureLossIndication => {
                if self
                    .send_video_picture_loss_indication(runtime_stats)
                    .is_ok()
                {
                    self.control_service.clear_pending_keyframe_request();
                    self.video_recovery_transport_state.stage =
                        VideoRecoveryTransportStage::PictureLossIndication;
                    self.video_recovery_transport_state.last_sent_at_ms =
                        Some(crate::transport::rtc::stats::now_ms_f64());
                    return Ok(());
                }
                self.send_control_keyframe_request(runtime_stats, "rtcVideoPliFallbackControl")
            }
            VideoRecoveryTransportStage::FullIntraRequest => {
                if self.send_video_full_intra_request(runtime_stats).is_ok() {
                    self.control_service.clear_pending_keyframe_request();
                    self.video_recovery_transport_state.stage =
                        VideoRecoveryTransportStage::FullIntraRequest;
                    self.video_recovery_transport_state.last_sent_at_ms =
                        Some(crate::transport::rtc::stats::now_ms_f64());
                    return Ok(());
                }
                self.send_control_keyframe_request(runtime_stats, "rtcVideoFirFallbackControl")
            }
            VideoRecoveryTransportStage::ControlKeyframe => {
                self.send_control_keyframe_request(runtime_stats, "rtcControlKeyframeRequested")
            }
        }
    }

    fn resolve_video_recovery_transport_stage(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> VideoRecoveryTransportStage {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let (current_epoch, clean_anchor_epoch, supports_pli, supports_fir) =
            RuntimeStatsSink::read_shared(runtime_stats, |stats| {
                let supported = stats
                    .latest_remote_answer_observation
                    .as_ref()
                    .map(|observation| {
                        let pli_supported = observation
                            .accepted_video_rtcp_feedback
                            .iter()
                            .any(|feedback| feedback == "nack:pli");
                        let fir_supported = observation
                            .accepted_video_rtcp_feedback
                            .iter()
                            .any(|feedback| feedback == "ccm:fir");
                        (pli_supported, fir_supported)
                    })
                    .unwrap_or((false, false));
                (
                    stats.transport_recovery_epoch,
                    stats.video_anchor_clean_epoch,
                    supported.0,
                    supported.1,
                )
            })
            .unwrap_or((0, None, false, false));

        if self.video_recovery_transport_state.recovery_epoch != current_epoch {
            self.video_recovery_transport_state = VideoRecoveryTransportState {
                recovery_epoch: current_epoch,
                ..Default::default()
            };
        }

        if clean_anchor_epoch == Some(current_epoch) {
            self.video_recovery_transport_state.stage = VideoRecoveryTransportStage::None;
            self.video_recovery_transport_state.last_sent_at_ms = None;
            return VideoRecoveryTransportStage::None;
        }

        let elapsed_ms = self
            .video_recovery_transport_state
            .last_sent_at_ms
            .map(|last| (now_ms - last).max(0.0))
            .unwrap_or(f64::INFINITY);
        match self.video_recovery_transport_state.stage {
            VideoRecoveryTransportStage::None => {
                if supports_pli {
                    VideoRecoveryTransportStage::PictureLossIndication
                } else if supports_fir {
                    VideoRecoveryTransportStage::FullIntraRequest
                } else {
                    VideoRecoveryTransportStage::ControlKeyframe
                }
            }
            VideoRecoveryTransportStage::PictureLossIndication => {
                if supports_fir && elapsed_ms >= VIDEO_RECOVERY_PLI_TO_FIR_MIN_DELAY_MS {
                    VideoRecoveryTransportStage::FullIntraRequest
                } else if !supports_fir && elapsed_ms >= VIDEO_RECOVERY_PLI_TO_FIR_MIN_DELAY_MS {
                    VideoRecoveryTransportStage::ControlKeyframe
                } else {
                    VideoRecoveryTransportStage::None
                }
            }
            VideoRecoveryTransportStage::FullIntraRequest => {
                if elapsed_ms >= VIDEO_RECOVERY_FIR_TO_CONTROL_MIN_DELAY_MS {
                    VideoRecoveryTransportStage::ControlKeyframe
                } else {
                    VideoRecoveryTransportStage::None
                }
            }
            VideoRecoveryTransportStage::ControlKeyframe => VideoRecoveryTransportStage::None,
        }
    }

    fn send_video_picture_loss_indication(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let Some((receiver_id, media_ssrc)) = self
            .controlled_twcc_feedback
            .preferred_video_feedback_target()
        else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineRtcVideoPliFeedbackTargetUnavailable",
            ));
        };
        let Some(media_ssrc) = media_ssrc else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineRtcVideoPliMediaSsrcUnavailable",
            ));
        };
        let Some(peer_connection) = self.peer_connection.as_mut() else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineRtcPeerConnectionUnavailable",
            ));
        };
        let Some(mut receiver) = peer_connection.rtp_receiver(receiver_id) else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineRtcReceiverLookupFailedForVideoPli",
            ));
        };
        let pli = PictureLossIndication {
            sender_ssrc: self.local_rtcp_sender_ssrc,
            media_ssrc,
        };
        receiver.write_rtcp(vec![Box::new(pli)]).map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcWriteVideoPliFailed: {err}"))
        })?;
        let sent_at_ms = crate::transport::rtc::stats::now_ms_f64();
        self.record_video_recovery_observation(
            runtime_stats,
            "rtcVideoPliRequested",
            &format!(
                "phase1 rtc video PLI requested mediaSsrc={media_ssrc} receiverId={receiver_id:?}",
            ),
        );
        RuntimeStatsSink::new(runtime_stats.clone()).record_keyframe_request_episode_sent(
            "pli",
            sent_at_ms,
            Some(sent_at_ms + KEYFRAME_REQUEST_RESPONSE_WINDOW_MS),
        );
        RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
            stats.video_pli_request_count_total =
                stats.video_pli_request_count_total.saturating_add(1);
        });
        Ok(())
    }

    fn send_video_full_intra_request(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let Some((receiver_id, media_ssrc)) = self
            .controlled_twcc_feedback
            .preferred_video_feedback_target()
        else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineRtcVideoFirFeedbackTargetUnavailable",
            ));
        };
        let Some(media_ssrc) = media_ssrc else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineRtcVideoFirMediaSsrcUnavailable",
            ));
        };
        let Some(peer_connection) = self.peer_connection.as_mut() else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineRtcPeerConnectionUnavailable",
            ));
        };
        let Some(mut receiver) = peer_connection.rtp_receiver(receiver_id) else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineRtcReceiverLookupFailedForVideoFir",
            ));
        };
        let fir = FullIntraRequest {
            sender_ssrc: self.local_rtcp_sender_ssrc,
            media_ssrc,
            fir: vec![FirEntry {
                ssrc: media_ssrc,
                sequence_number: 1,
            }],
        };
        receiver.write_rtcp(vec![Box::new(fir)]).map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcWriteVideoFirFailed: {err}"))
        })?;
        let sent_at_ms = crate::transport::rtc::stats::now_ms_f64();
        self.record_video_recovery_observation(
            runtime_stats,
            "rtcVideoFirRequested",
            &format!(
                "phase1 rtc video FIR requested mediaSsrc={media_ssrc} receiverId={receiver_id:?}",
            ),
        );
        RuntimeStatsSink::new(runtime_stats.clone()).record_keyframe_request_episode_sent(
            "fir",
            sent_at_ms,
            Some(sent_at_ms + KEYFRAME_REQUEST_RESPONSE_WINDOW_MS),
        );
        RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
            stats.video_pli_request_count_total =
                stats.video_pli_request_count_total.saturating_add(1);
        });
        Ok(())
    }

    fn send_control_keyframe_request(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        observation_label: &str,
    ) -> Result<(), XbxEngineRuntimeError> {
        let result = self.control_service.request_video_keyframe();
        self.video_recovery_transport_state.stage = VideoRecoveryTransportStage::ControlKeyframe;
        self.video_recovery_transport_state.last_sent_at_ms =
            Some(crate::transport::rtc::stats::now_ms_f64());
        if let Err(error) = result {
            self.record_video_recovery_observation(
                runtime_stats,
                observation_label,
                "phase1 rtc control keyframe queued for replay",
            );
            RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
                stats.video_pli_request_count_total =
                    stats.video_pli_request_count_total.saturating_add(1);
            });
            return Err(error);
        }
        self.send_control_payload(
            build_control_keyframe_request_payload(),
            observation_label,
            "phase1 rtc control keyframe requested",
            runtime_stats,
        )?;
        let sent_at_ms = crate::transport::rtc::stats::now_ms_f64();
        RuntimeStatsSink::new(runtime_stats.clone()).record_keyframe_request_episode_sent(
            "control",
            sent_at_ms,
            Some(sent_at_ms + KEYFRAME_REQUEST_RESPONSE_WINDOW_MS),
        );
        self.video_recovery_transport_state.stage = VideoRecoveryTransportStage::ControlKeyframe;
        self.video_recovery_transport_state.last_sent_at_ms = Some(sent_at_ms);
        RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
            stats.video_pli_request_count_total =
                stats.video_pli_request_count_total.saturating_add(1);
        });
        Ok(())
    }

    fn record_video_recovery_observation(
        &self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        label: &str,
        summary: &str,
    ) {
        RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
            stats.latest_observation_label = Some(label.to_string());
            stats.latest_observation_summary = Some(summary.to_string());
        });
    }

    fn sync_video_recovery_transport_state(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) {
        let current_epoch =
            RuntimeStatsSink::read_shared(runtime_stats, |stats| stats.transport_recovery_epoch)
                .unwrap_or(0);
        if self.video_recovery_transport_state.recovery_epoch != current_epoch {
            self.video_recovery_transport_state = VideoRecoveryTransportState {
                recovery_epoch: current_epoch,
                ..Default::default()
            };
        }
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
            sender_ssrc: self.local_rtcp_sender_ssrc,
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

    pub(crate) fn send_video_rtcp_payload(
        &mut self,
        payload: &[u8],
    ) -> Result<(), XbxEngineRuntimeError> {
        let Some((receiver_id, _media_ssrc)) = self
            .controlled_twcc_feedback
            .preferred_video_feedback_target()
        else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineRtcVideoRtcpFeedbackTargetUnavailable",
            ));
        };
        let Some(peer_connection) = self.peer_connection.as_mut() else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineRtcPeerConnectionUnavailable",
            ));
        };
        let Some(mut receiver) = peer_connection.rtp_receiver(receiver_id) else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineRtcReceiverLookupFailedForVideoRtcp",
            ));
        };

        let mut raw = bytes::Bytes::copy_from_slice(payload);
        let packets = rtc_rtcp::packet::unmarshal(&mut raw).map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcVideoRtcpParseFailed: {err}"))
        })?;
        receiver.write_rtcp(packets).map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcVideoRtcpWriteFailed: {err}"))
        })
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
            if self.should_refresh_target_remb(target_kbps) {
                let _ = self.request_target_remb_kbps(target_kbps, runtime_stats);
            }
        }
        crate::xbx_log_warn!("[xbxengine][rtc-connection] pump after drain peer events/reads");
        self.try_send_message_handshake(runtime_stats)?;
        self.run_delayed_control_actions(runtime_stats)?;
        RuntimeStatsSink::new(runtime_stats.clone())
            .record_keyframe_request_episode_timeout(crate::transport::rtc::stats::now_ms_f64());
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
    fn should_refresh_target_remb(&self, target_kbps: u32) -> bool {
        self.last_target_remb_requested_kbps != Some(target_kbps)
    }
}

pub(super) fn generate_local_rtcp_sender_ssrc() -> u32 {
    let seed = crate::transport::rtc::stats::now_ms_f64() as u32;
    if seed == 0 {
        1
    } else {
        seed
    }
}

#[cfg(test)]
#[path = "tests/service.rs"]
mod tests;
