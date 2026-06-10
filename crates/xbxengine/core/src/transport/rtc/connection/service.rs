use rtc::rtcp::payload_feedbacks::full_intra_request::{FirEntry, FullIntraRequest};
use rtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use rtc::rtcp::payload_feedbacks::receiver_estimated_maximum_bitrate::ReceiverEstimatedMaximumBitrate;
use std::sync::{Arc, Mutex};

use crate::api::runtime::XbxEngineWebRtcRuntimeConfig;
use crate::runtime_stats_sink::RuntimeStatsSink;
#[cfg(test)]
use crate::transport::rtc::connection::build_control_decoder_reset_payload;
#[allow(unused_imports)]
pub(super) use crate::transport::rtc::connection::builder::{
    build_owned_h264_codec_preferences, register_owned_h264_codecs, ControlledPeerConnection,
};
use crate::transport::rtc::connection::control_channel::RtcControlChannelService;
use crate::transport::rtc::connection::data_channel::{
    build_dimensions_changed_message_payload, StreamViewportDimensions, MESSAGE_CHANNEL_LABEL,
};
use crate::transport::rtc::connection::io_runtime::RtcIoRuntime;
use crate::transport::rtc::connection::runtime_state::RtcConnectionRuntimeState;
use crate::transport::rtc::connection::transport_metrics::describe_selected_candidate_pair;
use crate::transport::rtc::connection::twcc_feedback::ControlledTwccFeedbackController;
use crate::transport::rtc::events::RtcConnectionLifecycleState;
use crate::transport::rtc::facts::TransportFact;
use crate::transport::rtc::stream::{RtcMediaIngressPacket, RtcRtpPacketMeta};
use crate::{XbxEngineMediaRuntimeStats, XbxEngineRuntimeError};
use xbxengine_protocol::XbxEngineTargetTypeDto;

// 与恢复侧的 escalation window 对齐，作为首个图片恢复响应的统一观测窗口。
const PICTURE_RECOVERY_RESPONSE_WINDOW_MS: f64 = 960.0;
const TWCC_WARMUP_FEEDBACK_INTERVAL_MS: u64 = 50;
const TWCC_STABLE_FEEDBACK_INTERVAL_MS: u64 = 100;
pub(crate) const VIDEO_RTCP_FEEDBACK_TARGET_PENDING_REASON: &str = "videoRtcpFeedbackTargetPending";
const VIDEO_RTCP_FEEDBACK_TRANSPORT_NOT_READY_REASON: &str = "videoRtcpFeedbackTransportNotReady";
const LOW_RESOLUTION_REASSERT_MIN_TARGET_WIDTH: u32 = 1920;
const LOW_RESOLUTION_REASSERT_MIN_TARGET_HEIGHT: u32 = 1080;

/// PLI/FIR 依赖 TWCC 反馈目标；重连或轨未绑定时无目标，返回 pending 让上层按 deferred 语义处理。
fn video_rtcp_recovery_feedback_media_ssrc_ready(
    twcc: &mut ControlledTwccFeedbackController,
) -> bool {
    twcc.preferred_video_feedback_target()
        .and_then(|(_, media_ssrc)| media_ssrc)
        .is_some()
}

fn video_rtcp_transport_ready(service: &RtcConnectionService) -> bool {
    service.lifecycle_state == RtcConnectionLifecycleState::Connected
        && service.last_selected_pair_diagnostic.is_some()
}

pub(super) fn resolve_twcc_feedback_interval_target_ms(
    stats: &XbxEngineMediaRuntimeStats,
    configured_interval_ms: u64,
) -> u64 {
    let configured = configured_interval_ms.max(1);
    if stats.session_target_type != Some(XbxEngineTargetTypeDto::Cloud) {
        return configured;
    }
    if stats
        .latest_video_twcc_observation
        .as_ref()
        .is_some_and(|observation| {
            observation.source == "local-feedback" && observation.twcc_sample_valid
        })
    {
        return TWCC_STABLE_FEEDBACK_INTERVAL_MS;
    }
    let has_video_remote_twcc_binding = stats
        .latest_twcc_remote_stream_observation
        .as_ref()
        .is_some_and(|observation| {
            observation
                .mime_type
                .get(..5)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("video"))
        });
    let has_video_extension_signal = stats
        .latest_twcc_extension_observation
        .as_ref()
        .is_some_and(|observation| observation.state == "seen" || observation.state == "missing");
    if has_video_remote_twcc_binding || has_video_extension_signal {
        return TWCC_WARMUP_FEEDBACK_INTERVAL_MS;
    }
    configured
}
const GAMEPAD_RUMBLE_PENDING_TARGET_LIMIT: usize = 4;
const GAMEPAD_RUMBLE_DRAIN_PER_TICK_LIMIT: usize = 2;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum VideoRecoveryTransportStage {
    #[default]
    None,
    PictureLossIndication,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VideoRecoveryRequestOutcome {
    FeedbackTransportNotReady,
    FeedbackTargetPending,
    RequestedPli,
    RequestedControlKeyframe,
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
    pub(super) data_channel_catalog_observation_id: u64,
    pub(super) controlled_twcc_feedback: ControlledTwccFeedbackController,
    pub(super) pump_failure_injected: bool,
    pub(super) read_counters: RtcReadIngressCounters,
    pub(super) pending_media_ingress_packets:
        Vec<(RtcMediaIngressPacket, Option<RtcRtpPacketMeta>)>,
    pub(super) pending_gamepad_rumble_requests:
        Vec<ohmygamepad_protocol::OhMyGamepadRumbleRequestDto>,
    pub(super) pending_transport_facts: Vec<TransportFact>,
    pub(super) delayed_gamepad_added_due_at_ms: Option<f64>,
    pub(super) delayed_pli_prime_due_at_ms: Option<f64>,
    pub(super) pending_target_remb_kbps: Option<u32>,
    pub(super) active_target_remb_kbps: Option<u32>,
    pub(super) last_target_remb_request_at_ms: Option<f64>,
    pub(super) last_target_remb_requested_kbps: Option<u32>,
    pub(super) target_remb_request_count: u64,
    pub(super) local_rtcp_sender_ssrc: u32,
    pub(super) last_selected_pair_diagnostic: Option<String>,
    pub(super) selected_pair_snapshot_emitted: bool,
    pub(super) video_recovery_transport_state: VideoRecoveryTransportState,
    pub(super) last_low_resolution_recovery_config_change_rtp: Option<u32>,
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
            data_channel_catalog_observation_id: 0,
            controlled_twcc_feedback: ControlledTwccFeedbackController::new(
                webrtc_runtime_config.video_pipeline.feedback_interval_ms,
            ),
            pump_failure_injected: false,
            read_counters: RtcReadIngressCounters::default(),
            pending_media_ingress_packets: Vec::new(),
            pending_gamepad_rumble_requests: Vec::new(),
            pending_transport_facts: Vec::new(),
            delayed_gamepad_added_due_at_ms: None,
            delayed_pli_prime_due_at_ms: None,
            pending_target_remb_kbps: None,
            active_target_remb_kbps: None,
            last_target_remb_request_at_ms: None,
            last_target_remb_requested_kbps: None,
            target_remb_request_count: 0,
            local_rtcp_sender_ssrc: generate_local_rtcp_sender_ssrc(),
            last_selected_pair_diagnostic: None,
            selected_pair_snapshot_emitted: false,
            video_recovery_transport_state: VideoRecoveryTransportState::default(),
            last_low_resolution_recovery_config_change_rtp: None,
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
    fn maybe_adjust_twcc_feedback_interval(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) {
        let configured_interval_ms = self
            .webrtc_runtime_config
            .video_pipeline
            .feedback_interval_ms;
        let desired_interval_ms = RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            resolve_twcc_feedback_interval_target_ms(stats, configured_interval_ms)
        })
        .unwrap_or(configured_interval_ms.max(1));
        let current_interval_ms = self.controlled_twcc_feedback.feedback_interval_ms();
        if current_interval_ms == desired_interval_ms {
            return;
        }
        self.controlled_twcc_feedback
            .set_feedback_interval(desired_interval_ms);
        RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
            stats.latest_observation_label = Some("rtcTwccFeedbackIntervalAdjusted".to_string());
            stats.latest_observation_summary = Some(format!(
                "feedbackIntervalMs={} previousIntervalMs={}",
                desired_interval_ms, current_interval_ms
            ));
        });
    }

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

    /// receiver-local 关键帧请求：不写全局 recovery episode，不返回 recovery outcome。
    pub(crate) fn video_feedback_state(
        &mut self,
    ) -> crate::transport::rtc::capability::VideoFeedbackState {
        use crate::transport::rtc::capability::VideoFeedbackState;
        if !video_rtcp_transport_ready(self) {
            return VideoFeedbackState::Unavailable;
        }
        if video_rtcp_recovery_feedback_media_ssrc_ready(&mut self.controlled_twcc_feedback) {
            VideoFeedbackState::Ready
        } else {
            VideoFeedbackState::Warming
        }
    }

    pub(crate) fn video_keyframe_feedback_state(
        &mut self,
    ) -> crate::transport::rtc::capability::VideoFeedbackState {
        use crate::transport::rtc::capability::VideoFeedbackState;
        let rtcp_state = self.video_feedback_state();
        if matches!(rtcp_state, VideoFeedbackState::Ready) || self.control_keyframe_request_ready()
        {
            VideoFeedbackState::Ready
        } else {
            rtcp_state
        }
    }

    pub(crate) fn request_video_pli_direct(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        if !video_rtcp_transport_ready(self) {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineRtcVideoPliTransportNotReady",
            ));
        }
        if !video_rtcp_recovery_feedback_media_ssrc_ready(&mut self.controlled_twcc_feedback) {
            if let Some(receiver_id) = self
                .peer_connection
                .as_mut()
                .and_then(|peer_connection| peer_connection.get_receivers().next())
            {
                self.controlled_twcc_feedback
                    .prime_video_feedback_receiver_hint(receiver_id, runtime_stats);
            }
        }
        if !video_rtcp_recovery_feedback_media_ssrc_ready(&mut self.controlled_twcc_feedback) {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineRtcVideoPliFeedbackTargetUnavailable",
            ));
        }
        self.send_video_picture_loss_indication_direct(runtime_stats)
    }

    pub(crate) fn request_video_fir_direct(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        if !video_rtcp_transport_ready(self) {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineRtcVideoFirTransportNotReady",
            ));
        }
        self.send_video_full_intra_request_direct(runtime_stats)
    }

    pub(crate) fn request_video_pli_with_outcome(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<VideoRecoveryRequestOutcome, XbxEngineRuntimeError> {
        self.request_video_recovery_pli(runtime_stats)
    }

    #[cfg(test)]
    pub(crate) fn request_decoder_reset(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        if let Err(error) = self.control_service.request_decoder_reset() {
            self.sync_control_replay_runtime_stats(runtime_stats);
            return Err(error);
        }
        self.sync_control_replay_runtime_stats(runtime_stats);
        self.send_control_payload(
            build_control_decoder_reset_payload(),
            "rtcControlDecoderResetRequested",
            "phase1 rtc control decoder reset requested",
            runtime_stats,
        )
    }

    fn request_video_recovery_pli(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<VideoRecoveryRequestOutcome, XbxEngineRuntimeError> {
        self.sync_video_recovery_transport_state(runtime_stats);
        if !video_rtcp_transport_ready(self) {
            RuntimeStatsSink::new(runtime_stats.clone()).record_feedback_target_availability(
                crate::transport::rtc::stats::now_ms_f64(),
                "videoRtcpFeedback",
                "unbound",
                VIDEO_RTCP_FEEDBACK_TRANSPORT_NOT_READY_REASON,
            );
            self.sync_control_replay_runtime_stats(runtime_stats);
            return Ok(VideoRecoveryRequestOutcome::FeedbackTransportNotReady);
        }
        if !video_rtcp_recovery_feedback_media_ssrc_ready(&mut self.controlled_twcc_feedback) {
            if let Some(receiver_id) = self
                .peer_connection
                .as_mut()
                .and_then(|peer_connection| peer_connection.get_receivers().next())
            {
                self.controlled_twcc_feedback
                    .prime_video_feedback_receiver_hint(receiver_id, runtime_stats);
            }
        }
        if !video_rtcp_recovery_feedback_media_ssrc_ready(&mut self.controlled_twcc_feedback) {
            RuntimeStatsSink::new(runtime_stats.clone()).record_feedback_target_availability(
                crate::transport::rtc::stats::now_ms_f64(),
                "videoRtcpFeedback",
                "unbound",
                VIDEO_RTCP_FEEDBACK_TARGET_PENDING_REASON,
            );
            if self.control_keyframe_request_ready() {
                self.request_video_keyframe_control_direct(runtime_stats)?;
                self.sync_control_replay_runtime_stats(runtime_stats);
                return Ok(VideoRecoveryRequestOutcome::RequestedControlKeyframe);
            }
            self.sync_control_replay_runtime_stats(runtime_stats);
            return Ok(VideoRecoveryRequestOutcome::FeedbackTargetPending);
        }
        if self.control_keyframe_request_ready() {
            self.request_video_keyframe_control_direct(runtime_stats)?;
        }
        self.send_video_picture_loss_indication(runtime_stats)?;
        self.sync_control_replay_runtime_stats(runtime_stats);
        self.video_recovery_transport_state.stage =
            VideoRecoveryTransportStage::PictureLossIndication;
        self.video_recovery_transport_state.last_sent_at_ms =
            Some(crate::transport::rtc::stats::now_ms_f64());
        Ok(VideoRecoveryRequestOutcome::RequestedPli)
    }

    fn send_video_picture_loss_indication_direct(
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
        RuntimeStatsSink::new(runtime_stats.clone()).record_feedback_target_availability(
            sent_at_ms,
            "videoRtcpFeedback",
            "ready",
            "pliSent",
        );
        RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
            stats.video_pli_request_count_total =
                stats.video_pli_request_count_total.saturating_add(1);
        });
        Ok(())
    }

    fn send_video_full_intra_request_direct(
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
        RuntimeStatsSink::new(runtime_stats.clone()).record_feedback_target_availability(
            sent_at_ms,
            "videoRtcpFeedback",
            "ready",
            "firSent",
        );
        Ok(())
    }

    fn send_video_picture_loss_indication(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.send_video_picture_loss_indication_direct(runtime_stats)?;
        let sent_at_ms = crate::transport::rtc::stats::now_ms_f64();
        self.record_video_recovery_observation(
            runtime_stats,
            "rtcVideoPliRequested",
            "phase1 rtc video PLI requested (legacy recovery path)",
        );
        RuntimeStatsSink::new(runtime_stats.clone()).record_picture_recovery_episode_sent(
            "pli",
            sent_at_ms,
            Some(sent_at_ms + PICTURE_RECOVERY_RESPONSE_WINDOW_MS),
        );
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

    pub(super) fn sync_control_replay_runtime_stats(
        &self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) {
        let pending_count = self.control_service.pending_replay_action_count();
        let pending_since_ms = self.control_service.pending_replay_since_ms();
        RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
            stats.control_pending_replay_action_count = pending_count;
            stats.control_pending_replay_since_ms = pending_since_ms;
            stats.control_pending_replay_summary = if pending_count == 0 {
                None
            } else {
                Some(format!(
                    "decoderReset={} ready={}",
                    self.control_service.state().pending_decoder_reset,
                    self.control_service.is_control_ready()
                ))
            };
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
            self.video_recovery_transport_state.stage = VideoRecoveryTransportStage::None;
            self.video_recovery_transport_state.last_sent_at_ms = None;
        }
        self.video_recovery_transport_state.recovery_epoch = current_epoch;
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
        crate::xbx_log_debug!("[xbxengine][rtc-connection] pump enter");
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
                let pair_summary = pair_diagnostic.as_deref().unwrap_or("none");
                if pair_summary.starts_with("state=Succeeded") {
                    crate::xbx_log_warn!(
                        "[xbxengine][rtc-connection] selected_pair_snapshot {}",
                        pair_summary
                    );
                } else {
                    crate::xbx_log_debug!(
                        "[xbxengine][rtc-connection] selected_pair_snapshot {}",
                        pair_summary
                    );
                }
                self.last_selected_pair_diagnostic = pair_diagnostic;
                self.selected_pair_snapshot_emitted = true;
            }
        }
        crate::xbx_log_debug!("[xbxengine][rtc-connection] pump after io_runtime");
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
        crate::xbx_log_debug!("[xbxengine][rtc-connection] pump after drain peer events/reads");
        self.try_send_message_handshake(runtime_stats)?;
        self.maybe_adjust_twcc_feedback_interval(runtime_stats);
        self.maybe_reassert_runtime_resolution_after_low_resolution_recovery(runtime_stats)?;
        self.run_delayed_control_actions(runtime_stats)?;
        // delayed action 可能刚把 decoder reset pending 记入控制层，此处再尝试 flush。
        self.observe_control_replay_if_ready(runtime_stats)?;
        RuntimeStatsSink::new(runtime_stats.clone())
            .record_picture_recovery_episode_timeout(crate::transport::rtc::stats::now_ms_f64());
        self.refresh_transport_metrics(runtime_stats);
        crate::xbx_log_debug!("[xbxengine][rtc-connection] pump exit");
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
        let drain_count = self
            .pending_gamepad_rumble_requests
            .len()
            .min(GAMEPAD_RUMBLE_DRAIN_PER_TICK_LIMIT);
        self.pending_gamepad_rumble_requests
            .drain(..drain_count)
            .collect()
    }

    pub(crate) fn take_transport_facts(&mut self) -> Vec<TransportFact> {
        std::mem::take(&mut self.pending_transport_facts)
    }

    #[cfg(test)]
    pub(crate) fn inject_pump_failure(&mut self) {
        self.pump_failure_injected = true;
    }

    pub(crate) fn enqueue_pending_gamepad_rumble_requests(
        &mut self,
        requests: Vec<ohmygamepad_protocol::OhMyGamepadRumbleRequestDto>,
    ) {
        for request in requests {
            if let Some(existing_index) = self
                .pending_gamepad_rumble_requests
                .iter()
                .position(|existing| existing.target == request.target)
            {
                self.pending_gamepad_rumble_requests.remove(existing_index);
            }
            self.pending_gamepad_rumble_requests.push(request);
        }
        while self.pending_gamepad_rumble_requests.len() > GAMEPAD_RUMBLE_PENDING_TARGET_LIMIT {
            self.pending_gamepad_rumble_requests.remove(0);
        }
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

    pub(super) fn maybe_reassert_runtime_resolution_after_low_resolution_recovery(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let target_width = self
            .webrtc_runtime_config
            .negotiation
            .target_resolution_width
            .max(1);
        let target_height = self
            .webrtc_runtime_config
            .negotiation
            .target_resolution_height
            .max(1);
        if target_width < LOW_RESOLUTION_REASSERT_MIN_TARGET_WIDTH
            || target_height < LOW_RESOLUTION_REASSERT_MIN_TARGET_HEIGHT
        {
            return Ok(());
        }

        let Some((rtp_timestamp, observed_width, observed_height)) =
            RuntimeStatsSink::read_shared(runtime_stats, |stats| {
                let inspection = stats.latest_h264_inspection_observation.as_ref()?;
                if !inspection.is_idr
                    || !(inspection.config_changed || inspection.parameter_sets_changed)
                {
                    return None;
                }
                let observed_width = inspection.sample_width?;
                let observed_height = inspection.sample_height?;
                let rtp_timestamp = inspection.frame_rtp_timestamp?;
                if observed_width >= target_width && observed_height >= target_height {
                    return None;
                }
                Some((rtp_timestamp, observed_width, observed_height))
            })
            .flatten()
        else {
            return Ok(());
        };

        if self.last_low_resolution_recovery_config_change_rtp == Some(rtp_timestamp) {
            return Ok(());
        }
        self.last_low_resolution_recovery_config_change_rtp = Some(rtp_timestamp);

        let viewport = StreamViewportDimensions {
            width: target_width,
            height: target_height,
        };
        let dimensions_outcome = match self.data_channel_id_for_label(MESSAGE_CHANNEL_LABEL) {
            Some(channel_id) => match self.send_text_on_channel_id(
                channel_id,
                build_dimensions_changed_message_payload(viewport),
                "rtcRuntimeResolutionReasserted",
                &format!(
                    "runtime resolution reasserted observed={}x{} target={}x{} rtp={}",
                    observed_width, observed_height, target_width, target_height, rtp_timestamp
                ),
                runtime_stats,
            ) {
                Ok(()) => "sent".to_string(),
                Err(error) => format!("sendFailed({error})"),
            },
            None => "messageChannelMissing".to_string(),
        };

        let target_remb_kbps = desired_target_remb_kbps(runtime_stats)
            .or(self.active_target_remb_kbps)
            .unwrap_or_else(|| {
                self.webrtc_runtime_config
                    .remb_ceiling_kbps
                    .max(self.webrtc_runtime_config.negotiation.video_bitrate_kbps)
            });
        let remb_outcome = match self.request_target_remb_kbps(target_remb_kbps, runtime_stats) {
            Ok(()) => "requested".to_string(),
            Err(error) => format!("requestFailed({error})"),
        };
        let keyframe_outcome = match self.request_video_pli_with_outcome(runtime_stats) {
            Ok(outcome) => format!("{outcome:?}"),
            Err(error) => format!("requestFailed({error})"),
        };

        RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
            stats.latest_observation_label = Some("rtcRuntimeResolutionReasserted".to_string());
            stats.latest_observation_summary = Some(format!(
                "observed={}x{} target={}x{} rtp={} dimensions={} remb={}kbps:{} keyframe={}",
                observed_width,
                observed_height,
                target_width,
                target_height,
                rtp_timestamp,
                dimensions_outcome,
                target_remb_kbps,
                remb_outcome,
                keyframe_outcome
            ));
        });

        Ok(())
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
#[path = "service.test.rs"]
mod tests;
