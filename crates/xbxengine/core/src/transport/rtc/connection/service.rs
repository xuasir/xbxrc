use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::BytesMut;
use rtc::peer_connection::configuration::media_engine::{
    MediaEngine, MIME_TYPE_H264, MIME_TYPE_OPUS,
};
use rtc::peer_connection::configuration::{RTCConfigurationBuilder, RTCIceServer};
use rtc::peer_connection::event::{RTCDataChannelEvent, RTCPeerConnectionEvent};
use rtc::peer_connection::message::RTCMessage;
use rtc::peer_connection::sdp::RTCSessionDescription;
use rtc::peer_connection::state::{RTCIceGatheringState, RTCPeerConnectionState};
use rtc::peer_connection::transport::RTCIceCandidateInit;
use rtc::peer_connection::RTCPeerConnection;
use rtc::peer_connection::RTCPeerConnectionBuilder;
use rtc::rtp_transceiver::rtp_sender::{
    RTCPFeedback, RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters,
    RTCRtpEncodingParameters, RtpCodecKind,
};
use rtc::rtp_transceiver::{RTCRtpTransceiverDirection, RTCRtpTransceiverInit};
use rtc::sansio::Protocol;
use xbxengine_protocol::{
    XbxEngineIceCandidateDto, XbxEngineSessionDto, XbxEngineTransportStateDto,
};

use crate::api::runtime::XbxEngineNegotiationRuntimeConfig;
use crate::api::runtime::XbxEngineWebRtcRuntimeConfig;
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::bwe::evaluator::{RtcBweObservation, RtcBweState};
use crate::transport::rtc::connection::control_channel::RtcControlChannelService;
use crate::transport::rtc::connection::data_channel_bootstrap::{
    bootstrap_default_channels, build_control_authorization_payload,
    build_control_decoder_reset_payload, build_control_gamepad_changed_payload,
    build_control_keyframe_request_payload, build_input_metadata_bootstrap_packet,
    build_message_handshake_payload, build_post_handshake_message_payloads,
    is_handshake_ack_payload, CHAT_CHANNEL_LABEL, CONTROL_CHANNEL_LABEL, INPUT_CHANNEL_LABEL,
    MESSAGE_CHANNEL_LABEL,
};
use crate::transport::rtc::connection::io_runtime::RtcIoRuntime;
use crate::transport::rtc::connection::runtime_state::{
    RtcConnectionRuntimeState, RtcIceCandidateKind,
};
use crate::transport::rtc::connection::transport_metrics::{
    collect_transport_metrics, RtcTransportMetricsSnapshot,
};
use crate::transport::rtc::events::{RtcConnectionLifecycleState, RtcTransportEvent};
use crate::transport::rtc::media::{
    MediaPacketKind, RtcMediaIngressPacket, RtcMediaPacketSource, RtcRtpPacketMeta,
};
use crate::transport::rtc::stats::{apply_transport_event, now_ms_f64};
use crate::XbxEngineDataChannelMessageCatalogObservation;
use crate::{
    XbxEngineMediaRuntimeStats, XbxEnginePendingRuntimeRecoveryAction, XbxEngineRuntimeError,
};

const DEFAULT_ICE_SERVERS: [&str; 7] = [
    "stun:worldaz.relay.teams.microsoft.com:3478",
    "stun:stun.l.google.com:19302",
    "stun:stun1.l.google.com:19302",
    "stun:relay1.expressturn.com",
    "stun:relay2.expressturn.com",
    "stun:stun.kinesisvideo.us-east-1.amazonaws.com:443",
    "stun:stun.douyucdn.cn:18000",
];

pub(crate) struct RtcConnectionService {
    state: Arc<Mutex<RtcConnectionRuntimeState>>,
    peer_connection: Option<RTCPeerConnection>,
    io_runtime: RtcIoRuntime,
    control_service: RtcControlChannelService,
    webrtc_runtime_config: XbxEngineWebRtcRuntimeConfig,
    bwe_state: RtcBweState,
    bwe_stream_started_at: Instant,
    bwe_startup_grace: Duration,
    lifecycle_state: RtcConnectionLifecycleState,
    lifecycle_state_since_ms: f64,
    last_transport_metrics_sample_at_ms: f64,
    pending_runtime_recovery_action: Option<XbxEnginePendingRuntimeRecoveryAction>,
    lifecycle_observation_id: u64,
    pump_failure_injected: bool,
    read_counters: RtcReadIngressCounters,
    pending_media_ingress_packets: Vec<(RtcMediaIngressPacket, Option<RtcRtpPacketMeta>)>,
    delayed_gamepad_added_due_at_ms: Option<f64>,
    delayed_keyframe_prime_due_at_ms: Option<f64>,
}

impl Default for RtcConnectionService {
    fn default() -> Self {
        let webrtc_runtime_config = XbxEngineWebRtcRuntimeConfig::default();
        let bwe_startup_grace =
            Duration::from_millis(webrtc_runtime_config.recovery.first_frame_grace_ms);
        Self {
            state: Arc::new(Mutex::new(RtcConnectionRuntimeState::default())),
            peer_connection: None,
            io_runtime: RtcIoRuntime::default(),
            control_service: RtcControlChannelService::default(),
            webrtc_runtime_config: webrtc_runtime_config.clone(),
            bwe_state: RtcBweState::new(webrtc_runtime_config.remb_floor_kbps),
            bwe_stream_started_at: Instant::now(),
            bwe_startup_grace,
            lifecycle_state: RtcConnectionLifecycleState::Closed,
            lifecycle_state_since_ms: 0.0,
            last_transport_metrics_sample_at_ms: 0.0,
            pending_runtime_recovery_action: None,
            lifecycle_observation_id: 0,
            pump_failure_injected: false,
            read_counters: RtcReadIngressCounters::default(),
            pending_media_ingress_packets: Vec::new(),
            delayed_gamepad_added_due_at_ms: None,
            delayed_keyframe_prime_due_at_ms: None,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct RtcReadIngressCounters {
    rtp_packets: u64,
    rtcp_packets: u64,
    data_channel_messages: u64,
    last_data_channel_label: Option<String>,
}

const RTC_RECONNECT_GRACE_MS: f64 = 750.0;
const RTC_CONTROL_DELAYED_GAMEPAD_ADDED_MS: f64 = 500.0;
const RTC_CONTROL_DELAYED_KEYFRAME_PRIME_MS: f64 = 300.0;
const RTC_INPUT_BUFFERED_AMOUNT_HIGH_THRESHOLD_BYTES: u32 = 1024;
const RTC_INPUT_BUFFERED_AMOUNT_LOW_THRESHOLD_BYTES: u32 = 512;

impl RtcConnectionService {
    pub(crate) fn sync_runtime_config(&mut self, runtime_config: XbxEngineWebRtcRuntimeConfig) {
        self.webrtc_runtime_config = runtime_config;
        self.bwe_startup_grace =
            Duration::from_millis(self.webrtc_runtime_config.recovery.first_frame_grace_ms);
    }

    pub(crate) fn rebuild(
        &mut self,
        session: &XbxEngineSessionDto,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let preserved_remote_candidates = self.state.lock().ok().map(|state| {
            (
                state.remote_candidates.clone(),
                state.pending_remote_candidates.clone(),
                state.remote_candidate_keys.clone(),
                state.pending_remote_candidate_keys.clone(),
            )
        });
        if let Ok(mut state) = self.state.lock() {
            state.reset_for_session(session.target_type.clone());
            if let Some((
                remote_candidates,
                pending_remote_candidates,
                remote_candidate_keys,
                pending_remote_candidate_keys,
            )) = preserved_remote_candidates
            {
                state.remote_candidates = remote_candidates;
                state.pending_remote_candidates = pending_remote_candidates;
                state.remote_candidate_keys = remote_candidate_keys;
                state.pending_remote_candidate_keys = pending_remote_candidate_keys;
            }
        }
        self.peer_connection = None;
        self.io_runtime.rebuild()?;
        self.control_service.reset();
        self.pending_runtime_recovery_action = None;
        self.lifecycle_state = RtcConnectionLifecycleState::Connecting;
        self.lifecycle_state_since_ms = now_ms_f64();
        self.last_transport_metrics_sample_at_ms = 0.0;
        self.lifecycle_observation_id = self.lifecycle_observation_id.saturating_add(1);
        self.read_counters = RtcReadIngressCounters::default();
        self.pending_media_ingress_packets.clear();
        self.delayed_gamepad_added_due_at_ms = None;
        self.delayed_keyframe_prime_due_at_ms = None;
        self.bwe_state = RtcBweState::new(self.webrtc_runtime_config.remb_floor_kbps.max(1));
        self.bwe_stream_started_at = Instant::now();
        self.bwe_startup_grace =
            Duration::from_millis(self.webrtc_runtime_config.recovery.first_frame_grace_ms);
        let mut peer_connection = build_peer_connection(session)?;
        configure_offer_primitives(&mut peer_connection)?;
        if let Ok(mut state) = self.state.lock() {
            bootstrap_default_channels(&mut peer_connection, &mut state)?;
        }
        peer_connection
            .add_local_candidate(self.io_runtime.local_candidate()?)
            .map_err(|err| {
                XbxEngineRuntimeError::new(format!("xbxEngineRtcAddLocalCandidateFailed: {err}"))
            })?;
        self.peer_connection = Some(peer_connection);
        self.drain_peer_events(runtime_stats)?;
        self.publish_event(
            runtime_stats,
            RtcTransportEvent::ConnectionLifecycleChanged(RtcConnectionLifecycleState::Connecting),
        );
        Ok(())
    }

    pub(crate) fn create_raw_offer(
        &mut self,
        _negotiation: &XbxEngineNegotiationRuntimeConfig,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<String, XbxEngineRuntimeError> {
        let peer_connection = self
            .peer_connection
            .as_mut()
            .ok_or_else(|| XbxEngineRuntimeError::new("xbxEngineRtcPeerConnectionUnavailable"))?;
        let offer = peer_connection.create_offer(None).map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcCreateOfferFailed: {err}"))
        })?;
        let offer_sdp = offer.sdp.clone();
        peer_connection
            .set_local_description(offer)
            .map_err(|err| {
                XbxEngineRuntimeError::new(format!("xbxEngineRtcSetLocalDescriptionFailed: {err}"))
            })?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcConnectionStateLockFailed"))?;
        state.local_offer_sdp = Some(offer_sdp.clone());
        drop(state);
        self.pump(runtime_stats)?;
        self.publish_event(runtime_stats, RtcTransportEvent::LocalOfferCreated);
        Ok(offer_sdp)
    }

    pub(crate) fn apply_remote_description(
        &mut self,
        answer_sdp: &str,
        remote_candidates: &[XbxEngineIceCandidateDto],
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let peer_connection = self
            .peer_connection
            .as_mut()
            .ok_or_else(|| XbxEngineRuntimeError::new("xbxEngineRtcPeerConnectionUnavailable"))?;
        let answer = RTCSessionDescription::answer(answer_sdp.to_string()).map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcAnswerParseFailed: {err}"))
        })?;
        peer_connection
            .set_remote_description(answer)
            .map_err(|err| {
                XbxEngineRuntimeError::new(format!("xbxEngineRtcSetRemoteDescriptionFailed: {err}"))
            })?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcConnectionStateLockFailed"))?;
        state.remote_answer_sdp = Some(answer_sdp.to_string());
        if is_end_of_candidates_marker(answer_sdp) {
            state.record_remote_end_of_candidates();
        }
        let pending_remote_candidates = state.pending_remote_candidates.clone();
        state.pending_remote_candidates.clear();
        state.pending_remote_candidate_keys.clear();

        let mut merged_remote_candidates = Vec::new();
        let mut seen_keys = HashSet::new();
        for candidate in pending_remote_candidates
            .iter()
            .chain(remote_candidates.iter())
        {
            if is_end_of_candidates_candidate(candidate) {
                state.record_remote_end_of_candidates();
                continue;
            }
            let kind = classify_candidate_kind(&candidate.candidate);
            let key = candidate_identity_key(candidate);
            if !seen_keys.insert(key.clone()) {
                continue;
            }
            state.record_remote_candidate(candidate.clone(), kind, false, false);
            if !state.applied_remote_candidate_keys.contains(&key) {
                merged_remote_candidates.push((key, candidate.clone(), kind));
            }
        }
        drop(state);
        for (key, candidate, _kind) in merged_remote_candidates {
            add_remote_candidate_to_peer(peer_connection, &candidate)?;
            if let Ok(mut state) = self.state.lock() {
                state.applied_remote_candidate_keys.insert(key);
            }
        }
        self.pump(runtime_stats)?;
        self.publish_ice_snapshot(runtime_stats, "rtcRemoteDescriptionApplied");
        self.publish_event(runtime_stats, RtcTransportEvent::RemoteDescriptionApplied);
        Ok(())
    }

    pub(crate) fn add_remote_ice_candidates(
        &mut self,
        remote_candidates: &[XbxEngineIceCandidateDto],
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let mut should_cache_only = false;
        if let Some(peer_connection) = self.peer_connection.as_ref() {
            should_cache_only = peer_connection.remote_description().is_none();
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcConnectionStateLockFailed"))?;
        let mut candidates_to_apply = Vec::new();
        for candidate in remote_candidates {
            if is_end_of_candidates_candidate(candidate) {
                state.record_remote_end_of_candidates();
                continue;
            }
            let kind = classify_candidate_kind(&candidate.candidate);
            let key = candidate_identity_key(candidate);
            state.record_remote_candidate(candidate.clone(), kind, should_cache_only, false);
            if should_cache_only {
                continue;
            }
            if !state.applied_remote_candidate_keys.contains(&key) {
                candidates_to_apply.push((key, candidate.clone(), kind));
            }
        }
        if should_cache_only {
            drop(state);
        } else {
            drop(state);
            let peer_connection = self.peer_connection.as_mut().ok_or_else(|| {
                XbxEngineRuntimeError::new("xbxEngineRtcPeerConnectionUnavailable")
            })?;
            for (key, candidate, _kind) in candidates_to_apply {
                add_remote_candidate_to_peer(peer_connection, &candidate)?;
                if let Ok(mut state) = self.state.lock() {
                    state.applied_remote_candidate_keys.insert(key);
                }
            }
            self.pump(runtime_stats)?;
        }
        self.publish_ice_snapshot(runtime_stats, "rtcRemoteCandidateAdded");
        self.publish_event(runtime_stats, RtcTransportEvent::RemoteCandidateAdded);
        Ok(())
    }

    pub(crate) fn local_candidates_snapshot(&self) -> Vec<XbxEngineIceCandidateDto> {
        let candidates = self
            .state
            .lock()
            .ok()
            .map(|mut state| {
                if state.local_candidates.is_empty() {
                    if let Some(offer_sdp) = state.local_offer_sdp.clone() {
                        // offer SDP 里已经带出的 candidate 不能等到后续事件再补，
                        // 否则 runtime 会错过第一次 ICE 交换窗口。
                        for candidate in extract_local_candidates_from_offer_sdp(&offer_sdp) {
                            let kind = classify_candidate_kind(&candidate.candidate);
                            state.record_local_candidate(candidate, kind);
                        }
                    }
                }
                state.local_candidates.clone()
            })
            .unwrap_or_default();
        crate::xbx_log_warn!(
            "[xbxengine][rtc-connection] local_candidates_snapshot count={}",
            candidates.len()
        );
        candidates
    }

    pub(crate) fn local_ice_gathering_complete(&self) -> bool {
        let complete = self
            .state
            .lock()
            .ok()
            .is_some_and(|state| state.local_ice_gathering_complete);
        crate::xbx_log_warn!(
            "[xbxengine][rtc-connection] local_ice_gathering_complete complete={complete}"
        );
        complete
    }

    pub(crate) fn set_keyboard_pointer_enabled(&mut self, enabled: bool) {
        self.control_service.set_keyboard_pointer_enabled(enabled);
    }

    pub(crate) fn input_stream_ready(&self) -> bool {
        self.state.lock().ok().is_some_and(|state| {
            state.input_channel_open
                && state.input_metadata_bootstrapped
                && !state.input_backpressure_high
        })
    }

    pub(crate) fn send_input_stream_packet(
        &mut self,
        payload: Vec<u8>,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<bool, XbxEngineRuntimeError> {
        if !self.input_stream_ready() {
            return Ok(false);
        }
        let Some(channel_id) = self.data_channel_id_for_label(INPUT_CHANNEL_LABEL) else {
            return Ok(false);
        };
        let packet_len = payload.len();
        let seq = if payload.len() >= 6 {
            let mut seq_bytes = [0u8; 4];
            seq_bytes.copy_from_slice(&payload[2..6]);
            u32::from_le_bytes(seq_bytes)
        } else {
            0
        };
        let summary = format!("phase1 rtc input stream packet sent seq={seq} bytes={packet_len}");
        self.send_binary_on_channel_id(
            channel_id,
            payload,
            "rtcInputStreamPacketSent",
            &summary,
            runtime_stats,
        )?;
        Ok(true)
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

    pub(crate) fn stop(&mut self, runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>) {
        if let Some(peer_connection) = self.peer_connection.as_mut() {
            if let Err(err) = peer_connection.close() {
                RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
                    stats.latest_observation_label =
                        Some("rtcPeerConnectionCloseFailed".to_string());
                    stats.latest_observation_summary =
                        Some(format!("phase1 rtc peer connection close failed: {err}"));
                });
            }
        }
        self.peer_connection = None;
        self.io_runtime.stop();
        self.control_service.close_control_channel();
        self.control_service.close_message_channel();
        self.control_service.clear_pending_replay_actions();
        self.pending_runtime_recovery_action = None;
        self.lifecycle_state = RtcConnectionLifecycleState::Closed;
        self.lifecycle_state_since_ms = now_ms_f64();
        self.lifecycle_observation_id = self.lifecycle_observation_id.saturating_add(1);
        if let Ok(mut state) = self.state.lock() {
            *state = RtcConnectionRuntimeState::default();
        }
        self.delayed_gamepad_added_due_at_ms = None;
        self.delayed_keyframe_prime_due_at_ms = None;
        self.pending_media_ingress_packets.clear();
        self.publish_event(runtime_stats, RtcTransportEvent::TransportStopped);
        self.last_transport_metrics_sample_at_ms = 0.0;
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
        self.drain_peer_reads(runtime_stats)?;
        crate::xbx_log_warn!("[xbxengine][rtc-connection] pump after drain peer events/reads");
        self.try_send_message_handshake(runtime_stats)?;
        self.run_delayed_control_actions(runtime_stats)?;
        self.maybe_schedule_delayed_reconnect(runtime_stats);
        self.refresh_transport_metrics(runtime_stats);
        crate::xbx_log_warn!("[xbxengine][rtc-connection] pump exit");
        Ok(())
    }

    fn refresh_transport_metrics(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) {
        let now_ms = now_ms_f64();
        if now_ms - self.last_transport_metrics_sample_at_ms < 1_000.0 {
            return;
        }
        self.last_transport_metrics_sample_at_ms = now_ms;

        let Some(peer_connection) = self.peer_connection.as_mut() else {
            return;
        };

        let connected_at_ms =
            matches!(self.lifecycle_state, RtcConnectionLifecycleState::Connected)
                .then_some(self.lifecycle_state_since_ms);
        let Some(snapshot) = collect_transport_metrics(peer_connection, connected_at_ms) else {
            return;
        };
        let video_rtt_source = snapshot.video_rtt_source.clone();
        let transport_path = snapshot.transport_path.clone();

        let runtime_stats_sink = RuntimeStatsSink::new(runtime_stats.clone());
        runtime_stats_sink.record_transport_metrics(
            snapshot.video_rtt_ms,
            video_rtt_source,
            snapshot.inbound_video_loss_ratio_5s,
            snapshot.inbound_video_loss_ratio_1s,
            transport_path,
            snapshot.inbound_video_bitrate_kbps,
            snapshot.inbound_primary_video_bytes_total,
        );
        self.refresh_bandwidth_estimation(&runtime_stats_sink, &snapshot, now_ms);
    }

    fn refresh_bandwidth_estimation(
        &mut self,
        runtime_stats: &RuntimeStatsSink,
        snapshot: &RtcTransportMetricsSnapshot,
        now_ms: f64,
    ) {
        let observed_remb_kbps = runtime_stats
            .read(|stats| stats.video_remb_bps.map(|bps| bps / 1_000))
            .flatten();
        let bwe_mode = self.webrtc_runtime_config.bwe_mode.clone();
        let observation = RtcBweObservation {
            actual_kbps: snapshot.inbound_video_bitrate_kbps,
            fraction_lost: snapshot.inbound_video_loss_ratio_1s,
            rtt_ms: snapshot.video_rtt_ms,
            transport_path: snapshot.transport_path.clone(),
            observed_remb_kbps,
        };
        let evaluation = self.bwe_state.evaluate(
            runtime_stats,
            &self.webrtc_runtime_config,
            &observation,
            self.bwe_stream_started_at,
            self.bwe_startup_grace,
        );
        let target_remb_bps = evaluation.target_remb_kbps.saturating_mul(1_000);
        runtime_stats.update(|stats| {
            stats.session_phase = Some(evaluation.session_phase.as_str().to_string());
            stats.transport_policy_profile = Some(evaluation.transport_policy_profile.clone());
            stats.recovery_coupling_mode = Some(evaluation.recovery_coupling_mode.clone());
            stats.recovery_coupling_summary = Some(evaluation.recovery_coupling_summary.clone());
            stats.direct_gaming_bitrate_band = evaluation.direct_gaming_bitrate_band.clone();
            stats.video_remb_bps = Some(target_remb_bps);
            stats.latest_video_bwe_observation = Some(crate::XbxEngineVideoBweObservation {
                observation_id: evaluation.observation_id,
                mode: self.webrtc_runtime_config.bwe_mode.clone(),
                decision_reason: evaluation.decision_reason.clone(),
                target_remb_kbps: evaluation.target_remb_kbps,
                observed_remb_kbps,
                actual_video_bitrate_kbps: snapshot.inbound_video_bitrate_kbps,
                loss_ratio: snapshot.inbound_video_loss_ratio_1s,
                rtt_ms: snapshot.video_rtt_ms,
                transport_path: snapshot.transport_path.clone(),
                twcc_feedback_interval_ms: stats
                    .latest_video_twcc_observation
                    .as_ref()
                    .and_then(|twcc| twcc.feedback_interval_ms),
                twcc_observed_packet_count: stats
                    .latest_video_twcc_observation
                    .as_ref()
                    .map(|twcc| twcc.observed_packet_count),
                twcc_covered_sequence_span: stats
                    .latest_video_twcc_observation
                    .as_ref()
                    .map(|twcc| twcc.covered_sequence_span),
                twcc_receive_bitrate_kbps: stats
                    .latest_video_twcc_observation
                    .as_ref()
                    .and_then(|twcc| twcc.receive_bitrate_kbps),
                twcc_delivery_ratio: stats
                    .latest_video_twcc_observation
                    .as_ref()
                    .map(|twcc| twcc.delivery_ratio),
                twcc_loss_ratio: stats
                    .latest_video_twcc_observation
                    .as_ref()
                    .map(|twcc| twcc.packet_loss_ratio),
                observed_at_ms: now_ms,
            });
            stats.latest_observation_label = Some("rtcVideoBweEvaluated".to_string());
            stats.latest_observation_summary = Some(format!(
                "phase1 rtc bwe mode={} reason={} target={}kbps path={}",
                bwe_mode,
                evaluation.decision_reason,
                evaluation.target_remb_kbps,
                snapshot.transport_path.as_deref().unwrap_or("-"),
            ));
        });
    }

    pub(crate) fn take_media_ingress_packets(
        &mut self,
    ) -> Vec<(RtcMediaIngressPacket, Option<RtcRtpPacketMeta>)> {
        std::mem::take(&mut self.pending_media_ingress_packets)
    }

    pub(crate) fn take_pending_runtime_recovery_action(
        &mut self,
    ) -> Option<XbxEnginePendingRuntimeRecoveryAction> {
        self.pending_runtime_recovery_action.take()
    }

    #[cfg(test)]
    pub(crate) fn inject_pump_failure(&mut self) {
        self.pump_failure_injected = true;
    }

    fn handle_peer_connection_state_change(
        &mut self,
        state: RTCPeerConnectionState,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) {
        match state {
            RTCPeerConnectionState::Connected => {
                self.lifecycle_state = RtcConnectionLifecycleState::Connected;
                self.lifecycle_state_since_ms = now_ms_f64();
                self.pending_runtime_recovery_action = None;
                self.lifecycle_observation_id = self.lifecycle_observation_id.saturating_add(1);
                self.publish_lifecycle_observation(
                    runtime_stats,
                    RtcConnectionLifecycleState::Connected,
                    "phase1 rtc peer connection connected",
                    None,
                );
            }
            RTCPeerConnectionState::Connecting => {
                self.lifecycle_state = RtcConnectionLifecycleState::Connecting;
                self.lifecycle_state_since_ms = now_ms_f64();
                self.publish_lifecycle_observation(
                    runtime_stats,
                    RtcConnectionLifecycleState::Connecting,
                    "phase1 rtc peer connection connecting",
                    None,
                );
            }
            RTCPeerConnectionState::Disconnected => {
                self.lifecycle_state = RtcConnectionLifecycleState::Disconnected;
                self.lifecycle_state_since_ms = now_ms_f64();
                self.publish_lifecycle_observation(
                    runtime_stats,
                    RtcConnectionLifecycleState::Disconnected,
                    "phase1 rtc peer connection disconnected",
                    Some("transport disconnected".to_string()),
                );
            }
            RTCPeerConnectionState::Failed => {
                self.schedule_immediate_reconnect(
                    runtime_stats,
                    "rtcPeerConnectionFailed",
                    "phase1 rtc peer connection failed",
                    "peer connection failed",
                );
            }
            RTCPeerConnectionState::Closed => {
                self.schedule_immediate_reconnect(
                    runtime_stats,
                    "rtcPeerConnectionClosed",
                    "phase1 rtc peer connection closed",
                    "peer connection closed",
                );
            }
            _ => {
                self.lifecycle_state = RtcConnectionLifecycleState::Connecting;
                self.lifecycle_state_since_ms = now_ms_f64();
                self.publish_lifecycle_observation(
                    runtime_stats,
                    RtcConnectionLifecycleState::Connecting,
                    "phase1 rtc peer connection state changed",
                    Some(format!("rtc peer connection state changed: {state}")),
                );
            }
        }
    }

    fn schedule_immediate_reconnect(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        label: &str,
        summary: &str,
        reason: &str,
    ) {
        self.lifecycle_state = RtcConnectionLifecycleState::Recovering;
        self.lifecycle_state_since_ms = now_ms_f64();
        self.lifecycle_observation_id = self.lifecycle_observation_id.saturating_add(1);
        let mut created_recovery_action = false;
        if self.pending_runtime_recovery_action.is_none() {
            self.pending_runtime_recovery_action = Some(
                XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
                    observation_id: self.lifecycle_observation_id,
                    reason: reason.to_string(),
                },
            );
            created_recovery_action = true;
        }
        // 记录是否新建 recovery action，方便上层区分“瞬态 closed”还是“已进入恢复编排”。
        self.publish_lifecycle_observation(
            runtime_stats,
            RtcConnectionLifecycleState::Recovering,
            label,
            Some(format!(
                "{summary} reason={reason} recoveryActionCreated={created_recovery_action} observationId={}",
                self.lifecycle_observation_id
            )),
        );
    }

    fn maybe_schedule_delayed_reconnect(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) {
        if self.pending_runtime_recovery_action.is_some() {
            return;
        }
        let should_recover = matches!(
            self.lifecycle_state,
            RtcConnectionLifecycleState::Disconnected | RtcConnectionLifecycleState::Failed
        ) && now_ms_f64() - self.lifecycle_state_since_ms
            >= RTC_RECONNECT_GRACE_MS;
        if !should_recover {
            return;
        }
        let reason = match self.lifecycle_state {
            RtcConnectionLifecycleState::Disconnected => "peer connection disconnected",
            RtcConnectionLifecycleState::Failed => "peer connection failed",
            _ => "peer connection recovered",
        };
        self.schedule_immediate_reconnect(
            runtime_stats,
            "rtcConnectionRecovering",
            "phase1 rtc connection entering recovering",
            reason,
        );
    }

    fn publish_lifecycle_observation(
        &self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        lifecycle_state: RtcConnectionLifecycleState,
        label: &str,
        extra_summary: Option<String>,
    ) {
        let summary = match extra_summary {
            Some(extra) => format!(
                "phase1 rtc lifecycle={:?} state={:?} {extra}",
                lifecycle_state, self.lifecycle_state
            ),
            None => format!(
                "phase1 rtc lifecycle={:?} state={:?}",
                lifecycle_state, self.lifecycle_state
            ),
        };
        apply_transport_event(
            runtime_stats,
            lifecycle_state.transport_state(),
            label,
            &summary,
        );
    }

    fn mark_recovering_from_fault(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        label: &str,
        summary: &str,
        lifecycle_state: RtcConnectionLifecycleState,
        reason: String,
    ) {
        self.lifecycle_state = lifecycle_state;
        self.lifecycle_state_since_ms = now_ms_f64();
        self.lifecycle_observation_id = self.lifecycle_observation_id.saturating_add(1);
        if self.pending_runtime_recovery_action.is_none() {
            self.pending_runtime_recovery_action = Some(
                XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
                    observation_id: self.lifecycle_observation_id,
                    reason,
                },
            );
        }
        self.publish_lifecycle_observation(
            runtime_stats,
            RtcConnectionLifecycleState::Recovering,
            label,
            Some(summary.to_string()),
        );
    }

    fn drain_peer_events(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        if self.peer_connection.is_none() {
            return Ok(());
        }
        let mut pending_data_channel_events = Vec::new();
        let mut pending_ice_events = Vec::new();
        let mut pending_connection_states = Vec::new();
        let mut saw_local_candidate_update = false;
        let mut saw_local_gathering_complete = false;
        loop {
            let mut saw_event = false;
            {
                let Some(peer_connection) = self.peer_connection.as_mut() else {
                    return Ok(());
                };
                while let Some(event) = peer_connection.poll_event() {
                    saw_event = true;
                    match event {
                        RTCPeerConnectionEvent::OnIceCandidateEvent(ice_event) => {
                            pending_ice_events.push(ice_event);
                            saw_local_candidate_update = true;
                        }
                        RTCPeerConnectionEvent::OnIceGatheringStateChangeEvent(
                            RTCIceGatheringState::Complete,
                        ) => {
                            crate::xbx_log_warn!(
                                "[xbxengine][rtc-connection] ice gathering state complete observed"
                            );
                            if let Ok(mut state) = self.state.lock() {
                                state.record_local_end_of_candidates();
                            }
                            saw_local_gathering_complete = true;
                        }
                        RTCPeerConnectionEvent::OnConnectionStateChangeEvent(state) => {
                            pending_connection_states.push(state);
                        }
                        RTCPeerConnectionEvent::OnDataChannel(dc_event) => {
                            pending_data_channel_events.push(dc_event);
                        }
                        _ => {}
                    }
                }
            }
            for ice_event in pending_ice_events.drain(..) {
                self.record_local_candidate_event(ice_event, runtime_stats)?;
            }
            for state in pending_connection_states.drain(..) {
                self.handle_peer_connection_state_change(state, runtime_stats);
            }
            for event in pending_data_channel_events.drain(..) {
                self.apply_data_channel_event(event, runtime_stats)?;
            }
            if !saw_event {
                break;
            }
        }
        if saw_local_gathering_complete {
            self.publish_ice_snapshot(runtime_stats, "rtcLocalIceGatheringComplete");
        } else if saw_local_candidate_update {
            self.publish_ice_snapshot(runtime_stats, "rtcLocalIceCandidateObserved");
        }
        Ok(())
    }

    fn record_local_candidate_event(
        &self,
        ice_event: rtc::peer_connection::event::RTCPeerConnectionIceEvent,
        _runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let mut candidate = ice_event.candidate.to_json().map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcCandidateToJsonFailed: {err}"))
        })?;
        if candidate
            .sdp_mid
            .as_deref()
            .is_none_or(|mid| mid.trim().is_empty())
        {
            candidate.sdp_mid = Some("0".to_string());
        }
        if candidate.sdp_mline_index.is_none() {
            candidate.sdp_mline_index = Some(0);
        }
        let dto = XbxEngineIceCandidateDto {
            candidate: candidate.candidate,
            sdp_m_line_index: candidate.sdp_mline_index,
            sdp_mid: candidate.sdp_mid,
        };
        let kind = classify_candidate_kind(&dto.candidate);
        if let Ok(mut state) = self.state.lock() {
            state.record_local_candidate(dto, kind);
        } else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineRtcConnectionStateLockFailed",
            ));
        }
        Ok(())
    }

    fn publish_ice_snapshot(
        &self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        label: &str,
    ) {
        if let Ok(state) = self.state.lock() {
            RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
                stats.latest_observation_label = Some(label.to_string());
                stats.latest_observation_summary = Some(format!(
                    "phase1 rtc ice {}",
                    state.candidate_snapshot_summary()
                ));
            });
        }
    }

    fn apply_data_channel_event(
        &mut self,
        event: RTCDataChannelEvent,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        match event {
            RTCDataChannelEvent::OnOpen(channel_id) => {
                let label = self
                    .state
                    .lock()
                    .ok()
                    .and_then(|state| state.data_channel_labels.get(&channel_id).cloned());
                // 单独打出 OnOpen 观测，避免后续生命周期观测被状态摘要覆盖。
                crate::xbx_log_warn!(
                    "[xbxengine][rtc] data channel onopen observed channel_id={} label={}",
                    channel_id,
                    label.as_deref().unwrap_or("unknown")
                );
                match label.as_deref() {
                    Some(CONTROL_CHANNEL_LABEL) => {
                        self.control_service.open_control_channel();
                        crate::xbx_log_warn!(
                            "[xbxengine][rtc] rtcControlChannelOpened observed channel_id={}",
                            channel_id
                        );
                        self.publish_channel_lifecycle(
                            runtime_stats,
                            "rtcControlChannelOpened",
                            "phase1 rtc control channel opened",
                        );
                        self.try_bootstrap_control_channel(runtime_stats)?;
                    }
                    Some(MESSAGE_CHANNEL_LABEL) => {
                        self.control_service.open_message_channel();
                        crate::xbx_log_warn!(
                            "[xbxengine][rtc] rtcMessageChannelOpened observed channel_id={}",
                            channel_id
                        );
                        self.publish_channel_lifecycle(
                            runtime_stats,
                            "rtcMessageChannelOpened",
                            "phase1 rtc message channel opened",
                        );
                        self.try_send_message_handshake(runtime_stats)?;
                    }
                    Some(INPUT_CHANNEL_LABEL) => {
                        if let Ok(mut state) = self.state.lock() {
                            state.input_channel_open = true;
                            state.input_backpressure_high = false;
                        }
                        if let Some(peer_connection) = self.peer_connection.as_mut() {
                            if let Some(mut data_channel) = peer_connection.data_channel(channel_id)
                            {
                                data_channel.set_buffered_amount_high_threshold(
                                    RTC_INPUT_BUFFERED_AMOUNT_HIGH_THRESHOLD_BYTES,
                                );
                                data_channel.set_buffered_amount_low_threshold(
                                    RTC_INPUT_BUFFERED_AMOUNT_LOW_THRESHOLD_BYTES,
                                );
                            }
                        }
                        crate::xbx_log_warn!(
                            "[xbxengine][rtc] rtcInputChannelOpened observed channel_id={}",
                            channel_id
                        );
                        self.publish_channel_lifecycle(
                            runtime_stats,
                            "rtcInputChannelOpened",
                            "phase1 rtc input channel opened",
                        );
                        self.try_bootstrap_input_channel(runtime_stats)?;
                    }
                    Some(CHAT_CHANNEL_LABEL) => {
                        if let Ok(mut state) = self.state.lock() {
                            state.chat_channel_open = true;
                        }
                        crate::xbx_log_warn!(
                            "[xbxengine][rtc] rtcChatChannelOpened observed channel_id={}",
                            channel_id
                        );
                        self.publish_channel_lifecycle(
                            runtime_stats,
                            "rtcChatChannelOpened",
                            "phase1 rtc chat channel opened",
                        );
                    }
                    _ => {}
                }
                self.observe_control_replay_if_ready(runtime_stats)?;
            }
            RTCDataChannelEvent::OnClose(channel_id) => {
                let label = self
                    .state
                    .lock()
                    .ok()
                    .and_then(|state| state.data_channel_labels.get(&channel_id).cloned());
                match label.as_deref() {
                    Some(CONTROL_CHANNEL_LABEL) => {
                        self.control_service.close_control_channel();
                        self.delayed_gamepad_added_due_at_ms = None;
                        self.delayed_keyframe_prime_due_at_ms = None;
                        self.schedule_immediate_reconnect(
                            runtime_stats,
                            "rtcControlChannelClosed",
                            "phase1 rtc control channel closed",
                            "control channel closed",
                        );
                    }
                    Some(MESSAGE_CHANNEL_LABEL) => {
                        self.control_service.close_message_channel();
                        if let Ok(mut state) = self.state.lock() {
                            state.input_channel_open = false;
                            state.input_metadata_bootstrapped = false;
                            state.input_metadata_bootstrapped_after_handshake = false;
                            state.input_backpressure_high = false;
                            state.chat_channel_open = false;
                        }
                        self.delayed_gamepad_added_due_at_ms = None;
                        self.delayed_keyframe_prime_due_at_ms = None;
                        self.schedule_immediate_reconnect(
                            runtime_stats,
                            "rtcMessageChannelClosed",
                            "phase1 rtc message channel closed",
                            "message channel closed",
                        );
                    }
                    Some(INPUT_CHANNEL_LABEL) => {
                        if let Ok(mut state) = self.state.lock() {
                            state.input_channel_open = false;
                            state.input_metadata_bootstrapped = false;
                            state.input_metadata_bootstrapped_after_handshake = false;
                            state.input_backpressure_high = false;
                        }
                        self.publish_channel_lifecycle(
                            runtime_stats,
                            "rtcInputChannelClosed",
                            "phase1 rtc input channel closed",
                        );
                    }
                    Some(CHAT_CHANNEL_LABEL) => {
                        if let Ok(mut state) = self.state.lock() {
                            state.chat_channel_open = false;
                        }
                        self.publish_channel_lifecycle(
                            runtime_stats,
                            "rtcChatChannelClosed",
                            "phase1 rtc chat channel closed",
                        );
                    }
                    _ => {}
                }
            }
            RTCDataChannelEvent::OnBufferedAmountHigh(channel_id) => {
                let label = self
                    .state
                    .lock()
                    .ok()
                    .and_then(|state| state.data_channel_labels.get(&channel_id).cloned());
                if label.as_deref() == Some(INPUT_CHANNEL_LABEL) {
                    if let Ok(mut state) = self.state.lock() {
                        state.input_backpressure_high = true;
                    }
                    self.publish_channel_lifecycle(
                        runtime_stats,
                        "rtcInputBackpressureHigh",
                        "phase1 rtc input channel buffered amount high",
                    );
                }
            }
            RTCDataChannelEvent::OnBufferedAmountLow(channel_id) => {
                let label = self
                    .state
                    .lock()
                    .ok()
                    .and_then(|state| state.data_channel_labels.get(&channel_id).cloned());
                if label.as_deref() == Some(INPUT_CHANNEL_LABEL) {
                    if let Ok(mut state) = self.state.lock() {
                        state.input_backpressure_high = false;
                    }
                    self.publish_channel_lifecycle(
                        runtime_stats,
                        "rtcInputBackpressureLow",
                        "phase1 rtc input channel buffered amount low",
                    );
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn drain_peer_reads(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let Some(peer_connection) = self.peer_connection.as_mut() else {
            return Ok(());
        };
        let mut changed = false;
        let mut should_ack_message_handshake = false;
        let mut chat_text_observations = Vec::new();
        while let Some(message) = peer_connection.poll_read() {
            match message {
                RTCMessage::RtpPacket(track_id, packet) => {
                    self.read_counters.rtp_packets =
                        self.read_counters.rtp_packets.saturating_add(1);
                    self.pending_media_ingress_packets.push((
                        RtcMediaIngressPacket::new(
                            MediaPacketKind::Rtp,
                            packet.payload.len(),
                            RtcMediaPacketSource::Track {
                                track_id: format!("{track_id:?}"),
                            },
                        )
                        .with_rtp_payload(packet.payload.to_vec()),
                        Some(RtcRtpPacketMeta {
                            ssrc: packet.header.ssrc,
                            payload_type: packet.header.payload_type,
                            sequence_number: packet.header.sequence_number,
                            timestamp: packet.header.timestamp,
                            marker: packet.header.marker,
                        }),
                    ));
                    changed = true;
                }
                RTCMessage::RtcpPacket(track_id, packets) => {
                    self.read_counters.rtcp_packets =
                        self.read_counters.rtcp_packets.saturating_add(1);
                    let byte_len = packets.iter().map(|packet| packet.marshal_size()).sum();
                    self.pending_media_ingress_packets.push((
                        RtcMediaIngressPacket::new(
                            MediaPacketKind::Rtcp,
                            byte_len,
                            RtcMediaPacketSource::Track {
                                track_id: format!("{track_id:?}"),
                            },
                        ),
                        None,
                    ));
                    changed = true;
                }
                RTCMessage::DataChannelMessage(channel_id, payload) => {
                    self.read_counters.data_channel_messages =
                        self.read_counters.data_channel_messages.saturating_add(1);
                    let last_label = self
                        .state
                        .lock()
                        .ok()
                        .and_then(|state| state.data_channel_labels.get(&channel_id).cloned())
                        .unwrap_or_else(|| format!("id:{channel_id}"));
                    self.read_counters.last_data_channel_label = Some(last_label.clone());
                    if last_label == MESSAGE_CHANNEL_LABEL && payload.is_string {
                        let payload_text = String::from_utf8_lossy(payload.data.as_ref());
                        let preview = short_text_preview(payload_text.as_ref(), 96);
                        crate::xbx_log_warn!(
                            "[xbxengine][rtc] inbound message payload observed observation_id={} len={} preview={preview:?}",
                            self.read_counters.data_channel_messages,
                            payload_text.len()
                        );
                        if is_handshake_ack_payload(payload_text.as_ref()) {
                            crate::xbx_log_warn!(
                                "[xbxengine][rtc] inbound message handshake ack observed observation_id={} preview={preview:?}",
                                self.read_counters.data_channel_messages
                            );
                            should_ack_message_handshake = true;
                        }
                    } else if last_label == CHAT_CHANNEL_LABEL && payload.is_string {
                        let payload_text =
                            String::from_utf8_lossy(payload.data.as_ref()).to_string();
                        chat_text_observations
                            .push((self.read_counters.data_channel_messages, payload_text));
                    }
                    changed = true;
                }
            }
        }
        if should_ack_message_handshake {
            if self.control_service.ack_handshake() {
                self.send_post_handshake_messages(runtime_stats)?;
            }
            self.try_bootstrap_control_channel(runtime_stats)?;
            self.try_bootstrap_input_channel(runtime_stats)?;
            self.observe_control_replay_if_ready(runtime_stats)?;
        }
        if changed {
            let last_label = self
                .read_counters
                .last_data_channel_label
                .clone()
                .unwrap_or_else(|| "none".to_string());
            RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
                stats.latest_observation_label = Some("rtcReadIngressObserved".to_string());
                stats.latest_observation_summary = Some(format!(
                    "phase1 rtc read ingress rtp={} rtcp={} dc={} lastDc={}",
                    self.read_counters.rtp_packets,
                    self.read_counters.rtcp_packets,
                    self.read_counters.data_channel_messages,
                    last_label
                ));
                stats.latest_video_packet_arrival_time_ms = Some(now_ms_f64());
                if self.read_counters.data_channel_messages > 0 {
                    stats.latest_data_channel_message_catalog_observation =
                        Some(XbxEngineDataChannelMessageCatalogObservation {
                            observation_id: self.read_counters.data_channel_messages,
                            direction: "inbound".to_string(),
                            channel: last_label.clone(),
                            kind_type: Some("ingress".to_string()),
                            kind_message: Some("message".to_string()),
                            target: Some(last_label.clone()),
                            keys: vec!["channel".to_string()],
                            payload_len: 0,
                            observed_at_ms: now_ms_f64(),
                        });
                }
            });
            for (observation_id, payload_text) in chat_text_observations {
                self.record_chat_text_observation(observation_id, &payload_text, runtime_stats);
            }
        }
        Ok(())
    }

    fn observe_control_replay_if_ready(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let Some(actions) = self.control_service.peek_replay_actions_if_ready() else {
            return Ok(());
        };
        if actions.request_keyframe {
            self.send_control_payload(
                build_control_keyframe_request_payload(),
                "rtcControlReplayKeyframeSent",
                "phase1 rtc control replay keyframe sent",
                runtime_stats,
            )?;
        }
        if actions.request_decoder_reset {
            self.send_control_payload(
                build_control_decoder_reset_payload(),
                "rtcControlReplayDecoderResetSent",
                "phase1 rtc control replay decoder reset sent",
                runtime_stats,
            )?;
        }
        self.control_service.clear_pending_replay_actions();
        RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
            stats.latest_observation_label = Some("rtcControlReplayConsumed".to_string());
            stats.latest_observation_summary = Some(format!(
                "phase1 rtc control replay consumed keyframe={} decoderReset={}",
                actions.request_keyframe, actions.request_decoder_reset
            ));
        });
        Ok(())
    }

    fn try_send_message_handshake(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        if !self.control_service.should_send_message_handshake() {
            return Ok(());
        }
        let Some(channel_id) = self.data_channel_id_for_label(MESSAGE_CHANNEL_LABEL) else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineRtcMessageChannelMissing",
            ));
        };
        self.send_text_on_channel_id(
            channel_id,
            build_message_handshake_payload(),
            "rtcMessageHandshakeSent",
            "phase1 rtc message handshake sent",
            runtime_stats,
        )
    }

    fn send_post_handshake_messages(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        if !self.control_service.should_send_post_handshake_messages() {
            crate::xbx_log_warn!(
                "[xbxengine][rtc] post-handshake bootstrap skipped handshake_acked={} sent={}",
                self.control_service.state().message_handshake_acked,
                self.control_service.state().post_handshake_messages_sent
            );
            return Ok(());
        }
        let Some(channel_id) = self.data_channel_id_for_label(MESSAGE_CHANNEL_LABEL) else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineRtcMessageChannelMissing",
            ));
        };
        for payload in build_post_handshake_message_payloads() {
            crate::xbx_log_warn!(
                "[xbxengine][rtc] sending post-handshake payload channel_id={} len={}",
                channel_id,
                payload.len()
            );
            self.send_text_on_channel_id(
                channel_id,
                payload,
                "rtcMessagePostHandshakeSent",
                "phase1 rtc post-handshake message sent",
                runtime_stats,
            )?;
        }
        self.control_service.mark_post_handshake_messages_sent();
        Ok(())
    }

    fn try_bootstrap_control_channel(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        if !self.control_service.can_bootstrap_control() {
            let state = self.control_service.state();
            crate::xbx_log_warn!(
                "[xbxengine][rtc] control bootstrap skipped control_open={} control_started={} handshake_acked={} bootstrapped_after_handshake={}",
                state.control_channel_open,
                state.control_started,
                state.message_handshake_acked,
                state.control_bootstrapped_after_handshake
            );
            return Ok(());
        }
        crate::xbx_log_warn!(
            "[xbxengine][rtc] control bootstrap starting control_open={} control_started={} handshake_acked={} bootstrapped_after_handshake={}",
            self.control_service.state().control_channel_open,
            self.control_service.state().control_started,
            self.control_service.state().message_handshake_acked,
            self.control_service.state().control_bootstrapped_after_handshake
        );
        self.send_control_payload(
            build_control_authorization_payload(),
            "rtcControlAuthorizationSent",
            "phase1 rtc control authorization sent",
            runtime_stats,
        )?;
        self.send_control_payload(
            build_control_gamepad_changed_payload(false),
            "rtcControlGamepadRemovedSent",
            "phase1 rtc control gamepad removed sent",
            runtime_stats,
        )?;
        self.control_service.mark_control_bootstrapped();
        crate::xbx_log_warn!(
            "[xbxengine][rtc] control bootstrap completed control_open={} control_started={} handshake_acked={} bootstrapped_after_handshake={}",
            self.control_service.state().control_channel_open,
            self.control_service.state().control_started,
            self.control_service.state().message_handshake_acked,
            self.control_service.state().control_bootstrapped_after_handshake
        );
        let now_ms = now_ms_f64();
        self.delayed_gamepad_added_due_at_ms = Some(now_ms + RTC_CONTROL_DELAYED_GAMEPAD_ADDED_MS);
        self.delayed_keyframe_prime_due_at_ms =
            Some(now_ms + RTC_CONTROL_DELAYED_KEYFRAME_PRIME_MS);
        Ok(())
    }

    fn send_control_payload(
        &mut self,
        payload: String,
        observation_label: &str,
        observation_summary: &str,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let Some(channel_id) = self.data_channel_id_for_label(CONTROL_CHANNEL_LABEL) else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineRtcControlChannelMissing",
            ));
        };
        self.send_text_on_channel_id(
            channel_id,
            payload,
            observation_label,
            observation_summary,
            runtime_stats,
        )
    }

    fn send_text_on_channel_id(
        &mut self,
        channel_id: u16,
        payload: String,
        observation_label: &str,
        observation_summary: &str,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let peer_connection = self
            .peer_connection
            .as_mut()
            .ok_or_else(|| XbxEngineRuntimeError::new("xbxEngineRtcPeerConnectionUnavailable"))?;
        let mut data_channel = peer_connection.data_channel(channel_id).ok_or_else(|| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcDataChannelUnavailable({channel_id})"))
        })?;
        data_channel.send_text(payload).map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcDataChannelSendTextFailed: {err}"))
        })?;
        self.io_runtime.pump(peer_connection)?;
        RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
            stats.latest_observation_label = Some(observation_label.to_string());
            stats.latest_observation_summary = Some(observation_summary.to_string());
        });
        Ok(())
    }

    fn send_binary_on_channel_id(
        &mut self,
        channel_id: u16,
        payload: Vec<u8>,
        observation_label: &str,
        observation_summary: &str,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let peer_connection = self
            .peer_connection
            .as_mut()
            .ok_or_else(|| XbxEngineRuntimeError::new("xbxEngineRtcPeerConnectionUnavailable"))?;
        let mut data_channel = peer_connection.data_channel(channel_id).ok_or_else(|| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcDataChannelUnavailable({channel_id})"))
        })?;
        data_channel
            .send(BytesMut::from(payload.as_slice()))
            .map_err(|err| {
                XbxEngineRuntimeError::new(format!(
                    "xbxEngineRtcDataChannelSendBinaryFailed: {err}"
                ))
            })?;
        self.io_runtime.pump(peer_connection)?;
        RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
            stats.latest_observation_label = Some(observation_label.to_string());
            stats.latest_observation_summary = Some(observation_summary.to_string());
        });
        Ok(())
    }

    fn try_bootstrap_input_channel(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let Some(channel_id) = self.data_channel_id_for_label(INPUT_CHANNEL_LABEL) else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineRtcInputChannelMissing",
            ));
        };
        let handshake_acked = self.control_service.state().message_handshake_acked;
        let should_send = {
            let Ok(mut state) = self.state.lock() else {
                return Ok(());
            };
            if !state.input_channel_open {
                crate::xbx_log_warn!(
                    "[xbxengine][rtc] input bootstrap skipped because input channel is not open"
                );
                false
            } else {
                let should_send_pre_handshake = !state.input_metadata_bootstrapped;
                let should_send_post_handshake =
                    handshake_acked && !state.input_metadata_bootstrapped_after_handshake;
                if should_send_pre_handshake || should_send_post_handshake {
                    crate::xbx_log_warn!(
                        "[xbxengine][rtc] input bootstrap starting channel_open={} handshake_acked={} bootstrapped={} bootstrapped_after_handshake={}",
                        state.input_channel_open,
                        handshake_acked,
                        state.input_metadata_bootstrapped,
                        state.input_metadata_bootstrapped_after_handshake
                    );
                    state.input_metadata_bootstrapped = true;
                    if handshake_acked {
                        state.input_metadata_bootstrapped_after_handshake = true;
                    }
                    true
                } else {
                    crate::xbx_log_warn!(
                        "[xbxengine][rtc] input bootstrap skipped channel_open={} handshake_acked={} bootstrapped={} bootstrapped_after_handshake={}",
                        state.input_channel_open,
                        handshake_acked,
                        state.input_metadata_bootstrapped,
                        state.input_metadata_bootstrapped_after_handshake
                    );
                    false
                }
            }
        };
        if !should_send {
            return Ok(());
        }

        let packet = build_input_metadata_bootstrap_packet();
        let packet_len = packet.len();
        let summary = format!(
            "phase1 rtc input metadata bootstrap sent seq=0 maxTouchpoints=64 bytes={packet_len}"
        );
        crate::xbx_log_warn!(
            "[xbxengine][rtc] sending input metadata bootstrap channel_id={} bytes={}",
            channel_id,
            packet_len
        );
        match self.send_binary_on_channel_id(
            channel_id,
            packet,
            "rtcInputMetadataBootstrapSent",
            &summary,
            runtime_stats,
        ) {
            Ok(()) => Ok(()),
            Err(error) => {
                if let Ok(mut state) = self.state.lock() {
                    state.input_metadata_bootstrapped = false;
                    if handshake_acked {
                        state.input_metadata_bootstrapped_after_handshake = false;
                    }
                }
                Err(error)
            }
        }
    }

    fn publish_channel_lifecycle(
        &self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        label: &str,
        summary: &str,
    ) {
        let channel = if label.contains("Control") {
            "control"
        } else if label.contains("Message") {
            "message"
        } else if label.contains("Input") {
            "input"
        } else if label.contains("Chat") {
            "chat"
        } else {
            "unknown"
        };
        let lifecycle = if label.contains("Opened") {
            "open"
        } else if label.contains("Closed") {
            "close"
        } else {
            "lifecycle"
        };
        RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
            stats.latest_observation_label = Some(label.to_string());
            stats.latest_observation_summary = Some(summary.to_string());
            stats.latest_data_channel_message_catalog_observation =
                Some(XbxEngineDataChannelMessageCatalogObservation {
                    observation_id: self.read_counters.data_channel_messages,
                    direction: "local".to_string(),
                    channel: channel.to_string(),
                    kind_type: Some("lifecycle".to_string()),
                    kind_message: Some(lifecycle.to_string()),
                    target: Some(channel.to_string()),
                    keys: vec!["channel".to_string(), "state".to_string()],
                    payload_len: 0,
                    observed_at_ms: now_ms_f64(),
                });
        });
    }

    fn run_delayed_control_actions(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let now_ms = now_ms_f64();
        if self
            .delayed_gamepad_added_due_at_ms
            .is_some_and(|deadline| now_ms >= deadline)
        {
            self.delayed_gamepad_added_due_at_ms = None;
            self.send_control_payload(
                build_control_gamepad_changed_payload(true),
                "rtcControlGamepadAddedSent",
                "phase1 rtc control gamepad added sent",
                runtime_stats,
            )?;
        }
        if self
            .delayed_keyframe_prime_due_at_ms
            .is_some_and(|deadline| now_ms >= deadline)
        {
            self.delayed_keyframe_prime_due_at_ms = None;
            if self.control_service.is_control_ready() {
                self.send_control_payload(
                    build_control_keyframe_request_payload(),
                    "rtcControlDelayedKeyframePrimeSent",
                    "phase1 rtc delayed keyframe prime sent",
                    runtime_stats,
                )?;
            } else {
                let _ = self.control_service.request_video_keyframe();
                RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
                    stats.latest_observation_label =
                        Some("rtcControlDelayedKeyframePrimeDeferred".to_string());
                    stats.latest_observation_summary = Some(
                        "phase1 rtc delayed keyframe prime deferred until control ready"
                            .to_string(),
                    );
                });
            }
        }
        Ok(())
    }

    fn record_chat_text_observation(
        &self,
        observation_id: u64,
        payload_text: &str,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) {
        let preview = short_text_preview(payload_text, 48);
        RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
            stats.latest_data_channel_message_catalog_observation =
                Some(XbxEngineDataChannelMessageCatalogObservation {
                    observation_id,
                    direction: "inbound".to_string(),
                    channel: "chat".to_string(),
                    kind_type: None,
                    kind_message: Some("text".to_string()),
                    target: Some("chat".to_string()),
                    keys: vec!["text".to_string()],
                    payload_len: payload_text.len(),
                    observed_at_ms: now_ms_f64(),
                });
            stats.latest_observation_label = Some("rtcChatTextObserved".to_string());
            stats.latest_observation_summary = Some(format!(
                "phase1 rtc chat text observed len={} preview={preview:?}",
                payload_text.len()
            ));
        });
    }

    fn data_channel_id_for_label(&self, label: &str) -> Option<u16> {
        self.state.lock().ok().and_then(|state| {
            state
                .data_channel_labels
                .iter()
                .find_map(|(channel_id, channel_label)| {
                    (channel_label == label).then_some(*channel_id)
                })
        })
    }

    fn publish_event(
        &self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        event: RtcTransportEvent,
    ) {
        let ice_snapshot_summary = self
            .state
            .lock()
            .ok()
            .map(|state| state.candidate_snapshot_summary());
        match event {
            RtcTransportEvent::ConnectionLifecycleChanged(state) => apply_transport_event(
                runtime_stats,
                state.transport_state(),
                state.observation_label(),
                &format!("phase1 rtc connection lifecycle changed: {:?}", state),
            ),
            RtcTransportEvent::LocalOfferCreated => apply_transport_event(
                runtime_stats,
                XbxEngineTransportStateDto::Connecting,
                "rtcOfferCreated",
                &format!(
                    "phase1 rtc local offer created{}",
                    ice_snapshot_summary
                        .as_ref()
                        .map(|summary| format!(" ice={summary}"))
                        .unwrap_or_default()
                ),
            ),
            RtcTransportEvent::RemoteDescriptionApplied => apply_transport_event(
                runtime_stats,
                XbxEngineTransportStateDto::Connecting,
                "rtcRemoteDescriptionApplied",
                &format!(
                    "phase1 rtc remote description applied{}",
                    ice_snapshot_summary
                        .as_ref()
                        .map(|summary| format!(" ice={summary}"))
                        .unwrap_or_default()
                ),
            ),
            RtcTransportEvent::RemoteCandidateAdded => apply_transport_event(
                runtime_stats,
                XbxEngineTransportStateDto::Connecting,
                "rtcRemoteCandidateAdded",
                &format!(
                    "phase1 rtc remote candidate appended{}",
                    ice_snapshot_summary
                        .as_ref()
                        .map(|summary| format!(" ice={summary}"))
                        .unwrap_or_default()
                ),
            ),
            RtcTransportEvent::TransportStopped => apply_transport_event(
                runtime_stats,
                XbxEngineTransportStateDto::Closed,
                "rtcTransportStopped",
                "phase1 rtc transport stopped",
            ),
        }
    }
}

fn build_peer_connection(
    session: &XbxEngineSessionDto,
) -> Result<RTCPeerConnection, XbxEngineRuntimeError> {
    let mut media_engine = MediaEngine::default();
    media_engine.register_default_codecs().map_err(|err| {
        XbxEngineRuntimeError::new(format!("xbxEngineRtcRegisterDefaultCodecsFailed: {err}"))
    })?;
    // 对齐旧 webrtc 主线：在默认 codec 之外补齐我们稳定依赖的 H264 family。
    register_owned_h264_codecs(&mut media_engine)?;

    let mut ice_servers = Vec::new();
    if !cfg!(test) {
        ice_servers.push(RTCIceServer {
            urls: DEFAULT_ICE_SERVERS
                .iter()
                .map(|url| (*url).to_string())
                .collect(),
            ..Default::default()
        });
    }
    if let Some(turn_server) = session.turn_server.as_ref() {
        ice_servers.push(RTCIceServer {
            urls: vec![turn_server.url.clone()],
            username: turn_server.username.clone(),
            credential: turn_server.credential.clone(),
        });
    }
    let configuration = RTCConfigurationBuilder::new().with_ice_servers(ice_servers);
    RTCPeerConnectionBuilder::new()
        .with_configuration(configuration.build())
        .with_media_engine(media_engine)
        .build()
        .map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcBuildPeerConnectionFailed: {err}"))
        })
}

fn configure_offer_primitives(
    peer_connection: &mut RTCPeerConnection,
) -> Result<(), XbxEngineRuntimeError> {
    // 对齐旧 transport 的 offer 结构：audio + video + application 三段必须同时出现。
    peer_connection
        .add_transceiver_from_kind(
            RtpCodecKind::Audio,
            Some(RTCRtpTransceiverInit {
                direction: RTCRtpTransceiverDirection::Sendrecv,
                streams: vec![],
                send_encodings: vec![RTCRtpEncodingParameters {
                    // rtc 对 sendrecv 要求显式 base encoding，且 codec 必须能在 MediaEngine 命中。
                    rtp_coding_parameters: RTCRtpCodingParameters {
                        ssrc: Some(generate_offer_audio_ssrc()),
                        ..Default::default()
                    },
                    codec: RTCRtpCodec {
                        mime_type: MIME_TYPE_OPUS.to_string(),
                        clock_rate: 48_000,
                        channels: 2,
                        sdp_fmtp_line: "minptime=10;useinbandfec=1".to_string(),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
            }),
        )
        .map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcAddAudioTransceiverFailed: {err}"))
        })?;

    // rtc 0.9.0 没有公开的 transceiver.set_codec_preferences；
    // phase1 里通过 MediaEngine 预注册 + 上层 SDP policy 重排来显式约束视频偏好。
    peer_connection
        .add_transceiver_from_kind(
            RtpCodecKind::Video,
            Some(RTCRtpTransceiverInit {
                direction: RTCRtpTransceiverDirection::Recvonly,
                streams: vec![],
                send_encodings: vec![],
            }),
        )
        .map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcAddVideoTransceiverFailed: {err}"))
        })?;

    Ok(())
}

fn generate_offer_audio_ssrc() -> u32 {
    let seed = now_ms_f64() as u32;
    if seed == 0 {
        1
    } else {
        seed
    }
}

fn register_owned_h264_codecs(media_engine: &mut MediaEngine) -> Result<(), XbxEngineRuntimeError> {
    for codec in build_owned_h264_codec_preferences() {
        media_engine
            .register_codec(codec, RtpCodecKind::Video)
            .map_err(|err| {
                XbxEngineRuntimeError::new(format!(
                    "xbxEngineRtcRegisterOwnedH264CodecFailed: {err}"
                ))
            })?;
    }
    Ok(())
}

fn build_owned_h264_codec_preferences() -> Vec<RTCRtpCodecParameters> {
    let video_rtcp_feedback = vec![
        RTCPFeedback {
            typ: "goog-remb".to_string(),
            parameter: String::new(),
        },
        RTCPFeedback {
            typ: "transport-cc".to_string(),
            parameter: String::new(),
        },
        RTCPFeedback {
            typ: "ccm".to_string(),
            parameter: "fir".to_string(),
        },
        RTCPFeedback {
            typ: "nack".to_string(),
            parameter: String::new(),
        },
        RTCPFeedback {
            typ: "nack".to_string(),
            parameter: "pli".to_string(),
        },
    ];
    // 与旧主线一致：高 -> 主 -> 受限基线 -> 基线，最后附加 RTX(apt=124)。
    vec![
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: MIME_TYPE_H264.to_string(),
                clock_rate: 90_000,
                channels: 0,
                sdp_fmtp_line:
                    "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=640032"
                        .to_string(),
                rtcp_feedback: video_rtcp_feedback.clone(),
            },
            payload_type: 123,
        },
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: MIME_TYPE_H264.to_string(),
                clock_rate: 90_000,
                channels: 0,
                sdp_fmtp_line:
                    "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=4d0032"
                        .to_string(),
                rtcp_feedback: video_rtcp_feedback.clone(),
            },
            payload_type: 124,
        },
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: MIME_TYPE_H264.to_string(),
                clock_rate: 90_000,
                channels: 0,
                sdp_fmtp_line:
                    "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"
                        .to_string(),
                rtcp_feedback: video_rtcp_feedback.clone(),
            },
            payload_type: 125,
        },
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: MIME_TYPE_H264.to_string(),
                clock_rate: 90_000,
                channels: 0,
                sdp_fmtp_line:
                    "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42001f"
                        .to_string(),
                rtcp_feedback: video_rtcp_feedback,
            },
            payload_type: 102,
        },
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: "video/rtx".to_string(),
                clock_rate: 90_000,
                channels: 0,
                sdp_fmtp_line: "apt=124".to_string(),
                rtcp_feedback: vec![],
            },
            payload_type: 97,
        },
    ]
}

fn dto_to_rtc_candidate(candidate: &XbxEngineIceCandidateDto) -> RTCIceCandidateInit {
    RTCIceCandidateInit {
        candidate: candidate.candidate.clone(),
        sdp_mid: candidate.sdp_mid.clone(),
        sdp_mline_index: candidate.sdp_m_line_index,
        username_fragment: None,
        url: None,
    }
}

fn add_remote_candidate_to_peer(
    peer_connection: &mut RTCPeerConnection,
    candidate: &XbxEngineIceCandidateDto,
) -> Result<(), XbxEngineRuntimeError> {
    peer_connection
        .add_remote_candidate(dto_to_rtc_candidate(candidate))
        .map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcAddRemoteCandidateFailed: {err}"))
        })
}

fn classify_candidate_kind(candidate: &str) -> RtcIceCandidateKind {
    let mut tokens = candidate
        .split_whitespace()
        .map(|token| token.to_ascii_lowercase());
    while let Some(token) = tokens.next() {
        if token == "typ" {
            return match tokens.next().as_deref() {
                Some("host") => RtcIceCandidateKind::Host,
                Some("srflx") => RtcIceCandidateKind::Srflx,
                Some("relay") => RtcIceCandidateKind::Relay,
                _ => RtcIceCandidateKind::Unknown,
            };
        }
    }
    RtcIceCandidateKind::Unknown
}

fn is_end_of_candidates_candidate(candidate: &XbxEngineIceCandidateDto) -> bool {
    let trimmed = candidate.candidate.trim();
    trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("a=end-of-candidates")
        || trimmed.eq_ignore_ascii_case("end-of-candidates")
}

fn is_end_of_candidates_marker(sdp: &str) -> bool {
    sdp.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.eq_ignore_ascii_case("a=end-of-candidates")
            || trimmed.eq_ignore_ascii_case("end-of-candidates")
    })
}

fn extract_local_candidates_from_offer_sdp(offer_sdp: &str) -> Vec<XbxEngineIceCandidateDto> {
    let mut candidates = Vec::new();
    let mut current_mid: Option<String> = None;
    let mut current_mline_index: Option<u16> = None;

    for line in offer_sdp.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("m=") {
            current_mline_index = Some(current_mline_index.map_or(0, |index| index.saturating_add(1)));
            current_mid = None;
            continue;
        }

        if let Some(mid) = trimmed
            .strip_prefix("a=mid:")
            .or_else(|| trimmed.strip_prefix("mid:"))
        {
            let mid = mid.trim();
            if !mid.is_empty() {
                current_mid = Some(mid.to_string());
            }
            continue;
        }

        if trimmed.is_empty()
            || trimmed.eq_ignore_ascii_case("a=end-of-candidates")
            || trimmed.eq_ignore_ascii_case("end-of-candidates")
        {
            continue;
        }

        let Some(candidate) = trimmed
            .strip_prefix("a=candidate:")
            .or_else(|| trimmed.strip_prefix("candidate:"))
        else {
            continue;
        };

        candidates.push(XbxEngineIceCandidateDto {
            candidate: format!("candidate:{candidate}"),
            sdp_m_line_index: Some(current_mline_index.unwrap_or(0)),
            sdp_mid: current_mid
                .clone()
                .or_else(|| current_mline_index.map(|index| index.to_string())),
        });
    }

    candidates
}

fn candidate_identity_key(candidate: &XbxEngineIceCandidateDto) -> String {
    format!(
        "{}|{:?}|{}",
        candidate.candidate,
        candidate.sdp_m_line_index,
        candidate.sdp_mid.as_deref().unwrap_or("")
    )
}

fn short_text_preview(payload: &str, max_chars: usize) -> String {
    let mut preview = payload.chars().take(max_chars).collect::<String>();
    if payload.chars().count() > max_chars {
        preview.push_str("...");
    }
    preview
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;
    use std::net::{SocketAddr, UdpSocket};
    use std::thread;
    use std::time::{Duration, Instant};

    use bytes::BytesMut;
    use rtc::peer_connection::configuration::media_engine::MediaEngine;
    use rtc::peer_connection::configuration::RTCConfigurationBuilder;
    use rtc::peer_connection::event::{RTCDataChannelEvent, RTCPeerConnectionEvent};
    use rtc::peer_connection::sdp::RTCSessionDescription;
    use rtc::peer_connection::state::RTCPeerConnectionState;
    use rtc::peer_connection::transport::{
        CandidateConfig, CandidateHostConfig, RTCIceCandidate, RTCIceCandidateInit,
    };
    use rtc::peer_connection::RTCPeerConnection;
    use rtc::peer_connection::RTCPeerConnectionBuilder;
    use rtc::sansio::Protocol;
    use rtc::shared::{TaggedBytesMut, TransportContext, TransportProtocol};

    use super::{
        build_owned_h264_codec_preferences, dto_to_rtc_candidate, register_owned_h264_codecs,
        RtcConnectionLifecycleState, RtcConnectionService, CHAT_CHANNEL_LABEL,
        CONTROL_CHANNEL_LABEL, INPUT_CHANNEL_LABEL, MESSAGE_CHANNEL_LABEL,
    };
    use crate::api::runtime::XbxEngineWebRtcRuntimeConfig;
    use crate::runtime_stats_sink::RuntimeStatsSink;
    use crate::transport::rtc::connection::transport_metrics::RtcTransportMetricsSnapshot;
    use crate::{XbxEngineMediaRuntimeStats, XbxEngineRuntimeError, XbxEngineVideoTwccObservation};
    use std::sync::{Arc, Mutex};
    use xbxengine_protocol::{
        XbxEngineIceCandidateDto, XbxEngineSessionDto, XbxEngineTargetTypeDto,
    };

    const HANDSHAKE_ACK_PAYLOAD: &str = r#"{"type":"HandshakeAck"}"#;

    #[test]
    fn create_raw_offer_comes_from_real_rtc_peer_connection() {
        let mut service = RtcConnectionService::default();
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let session = XbxEngineSessionDto {
            session_id: "test-session".to_string(),
            target_type: XbxEngineTargetTypeDto::Cloud,
            turn_server: None,
        };

        service.rebuild(&session, &runtime_stats).unwrap();
        let offer = service
            .create_raw_offer(&Default::default(), &runtime_stats)
            .unwrap();

        assert!(offer.contains("m=audio"));
        assert!(offer.contains("m=video"));
        assert!(offer.contains("m=application"));
        assert!(offer.contains("webrtc-datachannel"));
        assert!(!service.local_candidates_snapshot().is_empty());
        let state = service.state.lock().expect("connection state");
        assert_eq!(state.local_candidate_host_count, 1);
        // 移除 eager EOC 注入后，gathering 完成由底层事件驱动，不再要求这里立即完成。
        assert!(state.local_candidate_end_of_candidates_count <= 1);
        drop(state);
        assert!(runtime_stats
            .lock()
            .expect("runtime stats")
            .latest_observation_summary
            .as_deref()
            .is_some_and(|summary| summary.contains("local total=1")));
    }

    #[test]
    fn local_candidates_snapshot_falls_back_to_offer_sdp_candidates() {
        let mut service = RtcConnectionService::default();
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let session = XbxEngineSessionDto {
            session_id: "test-session".to_string(),
            target_type: XbxEngineTargetTypeDto::Cloud,
            turn_server: None,
        };

        service.rebuild(&session, &runtime_stats).unwrap();
        let offer = service
            .create_raw_offer(&Default::default(), &runtime_stats)
            .unwrap();
        {
            let mut state = service.state.lock().expect("connection state");
            state.local_candidates.clear();
            state.local_candidate_keys.clear();
            state.local_candidate_count_total = 0;
            state.local_candidate_host_count = 0;
            state.local_candidate_srflx_count = 0;
            state.local_candidate_relay_count = 0;
            state.local_candidate_unknown_count = 0;
            state.latest_local_candidate_kind = None;
            state.latest_local_candidate_key = None;
            state.local_ice_gathering_complete = false;
            state.local_offer_sdp = Some(offer);
        }

        let candidates = service.local_candidates_snapshot();

        assert!(!candidates.is_empty());
        assert!(candidates
            .iter()
            .all(|candidate| candidate.candidate.starts_with("candidate:")));
        assert_eq!(
            service
                .state
                .lock()
                .expect("connection state")
                .local_candidate_count_total,
            candidates.len() as u64
        );
    }

    #[test]
    fn owned_h264_codec_preferences_include_main_profile_and_rtx_probe() {
        let codecs = build_owned_h264_codec_preferences();
        assert!(codecs.iter().any(|codec| {
            codec.payload_type == 124
                && codec.rtp_codec.mime_type.eq_ignore_ascii_case("video/h264")
                && codec
                    .rtp_codec
                    .sdp_fmtp_line
                    .contains("profile-level-id=4d0032")
        }));
        assert!(codecs.iter().any(|codec| {
            codec.payload_type == 97
                && codec.rtp_codec.mime_type.eq_ignore_ascii_case("video/rtx")
                && codec.rtp_codec.sdp_fmtp_line == "apt=124"
        }));
    }

    #[test]
    fn register_owned_h264_codecs_is_compatible_with_default_registry() {
        let mut media_engine = MediaEngine::default();
        media_engine.register_default_codecs().unwrap();
        // rtc::MediaEngine 的 codec 列表对外不可见；这里至少保证补充注册不会报错。
        assert!(register_owned_h264_codecs(&mut media_engine).is_ok());
    }

    #[test]
    fn refresh_transport_metrics_publishes_bwe_observation() {
        let mut service = RtcConnectionService::default();
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        {
            let mut config = XbxEngineWebRtcRuntimeConfig::default();
            config.bwe_mode = "hybrid".to_string();
            config.forced_remb_kbps = Some(24_000);
            config.adaptive_remb_enabled = true;
            config.remb_floor_kbps = 12_000;
            config.remb_ceiling_kbps = 60_000;
            config.remb_ramp_up_step_kbps = 3_000;
            config.remb_ramp_down_factor = 750;
            service.sync_runtime_config(config);
        }
        {
            let mut stats = runtime_stats.lock().unwrap();
            stats.session_target_type = Some(XbxEngineTargetTypeDto::Home);
            stats.latest_video_twcc_observation = Some(XbxEngineVideoTwccObservation {
                observation_id: 1,
                feedback_packet_count: 4,
                covered_sequence_start: 1,
                covered_sequence_end: 120,
                covered_sequence_span: 120,
                observed_packet_count: 120,
                observed_byte_count: 150_000,
                feedback_interval_ms: Some(100.0),
                arrival_span_ms: Some(100.0),
                receive_bitrate_kbps: Some(13_500.0),
                delivery_ratio: 1.0,
                packet_loss_ratio: 0.0,
                observed_at_ms: 1.0,
            });
        }
        let sink = RuntimeStatsSink::new(runtime_stats.clone());
        let snapshot = RtcTransportMetricsSnapshot {
            video_rtt_ms: Some(48.0),
            video_rtt_source: Some("candidate-pair".to_string()),
            inbound_video_loss_ratio_5s: 0.0,
            inbound_video_loss_ratio_1s: 0.0,
            transport_path: Some("Direct (host->host)".to_string()),
            inbound_video_bitrate_kbps: 11_500.0,
            inbound_primary_video_bytes_total: 900_000,
        };

        service.refresh_bandwidth_estimation(&sink, &snapshot, 1_234.0);

        let stats = runtime_stats.lock().unwrap();
        let bwe = stats
            .latest_video_bwe_observation
            .as_ref()
            .expect("bwe observation should be published");
        assert_eq!(bwe.mode, "hybrid");
        assert_eq!(
            stats.video_remb_bps,
            Some(bwe.target_remb_kbps.saturating_mul(1_000))
        );
        assert_eq!(
            stats.latest_observation_label.as_deref(),
            Some("rtcVideoBweEvaluated")
        );
        assert!(bwe.decision_reason.contains("twcc-gcc") || bwe.decision_reason.contains("hybrid"));
        assert_eq!(bwe.transport_path.as_deref(), Some("Direct (host->host)"));
    }

    #[test]
    fn apply_remote_description_accepts_real_rtc_answer() {
        let mut service = RtcConnectionService::default();
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let session = XbxEngineSessionDto {
            session_id: "test-session".to_string(),
            target_type: XbxEngineTargetTypeDto::Home,
            turn_server: None,
        };
        service.rebuild(&session, &runtime_stats).unwrap();
        let offer = service
            .create_raw_offer(&Default::default(), &runtime_stats)
            .unwrap();

        let mut answer_pc = RTCPeerConnectionBuilder::new()
            .with_configuration(RTCConfigurationBuilder::new().build())
            .build()
            .unwrap();
        answer_pc
            .set_remote_description(RTCSessionDescription::offer(offer).unwrap())
            .unwrap();
        let answer = answer_pc.create_answer(None).unwrap();
        answer_pc.set_local_description(answer.clone()).unwrap();

        service
            .apply_remote_description(&answer.sdp, &[], &runtime_stats)
            .unwrap();
    }

    #[test]
    fn add_remote_ice_candidates_deduplicates_when_remote_description_missing() {
        let mut service = RtcConnectionService::default();
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let session = XbxEngineSessionDto {
            session_id: "test-session".to_string(),
            target_type: XbxEngineTargetTypeDto::Cloud,
            turn_server: None,
        };
        service.rebuild(&session, &runtime_stats).unwrap();
        let duplicate = service
            .local_candidates_snapshot()
            .into_iter()
            .next()
            .expect("service local candidate");

        service
            .add_remote_ice_candidates(&[duplicate.clone(), duplicate], &runtime_stats)
            .unwrap();

        let state = service.state.lock().expect("connection state");
        assert_eq!(state.remote_candidates.len(), 1);
        assert_eq!(state.pending_remote_candidates.len(), 1);
        assert_eq!(state.remote_candidate_keys.len(), 1);
        assert_eq!(state.pending_remote_candidate_keys.len(), 1);
    }

    #[test]
    fn apply_remote_description_deduplicates_pending_and_inline_candidates() {
        let mut service = RtcConnectionService::default();
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let session = XbxEngineSessionDto {
            session_id: "test-session".to_string(),
            target_type: XbxEngineTargetTypeDto::Cloud,
            turn_server: None,
        };

        service.rebuild(&session, &runtime_stats).unwrap();
        let offer = service
            .create_raw_offer(&Default::default(), &runtime_stats)
            .unwrap();
        let duplicate = service
            .local_candidates_snapshot()
            .into_iter()
            .next()
            .expect("service local candidate");
        service
            .add_remote_ice_candidates(std::slice::from_ref(&duplicate), &runtime_stats)
            .unwrap();

        let mut answer_pc = RTCPeerConnectionBuilder::new()
            .with_configuration(RTCConfigurationBuilder::new().build())
            .build()
            .unwrap();
        answer_pc
            .set_remote_description(RTCSessionDescription::offer(offer).unwrap())
            .unwrap();
        let answer = answer_pc.create_answer(None).unwrap();
        answer_pc.set_local_description(answer.clone()).unwrap();

        service
            .apply_remote_description(&answer.sdp, &[duplicate.clone(), duplicate], &runtime_stats)
            .unwrap();

        let state = service.state.lock().expect("connection state");
        assert!(state.pending_remote_candidates.is_empty());
        assert!(state.pending_remote_candidate_keys.is_empty());
        assert_eq!(state.remote_candidate_keys.len(), 1);
        assert_eq!(state.applied_remote_candidate_keys.len(), 1);
    }

    #[test]
    fn add_remote_ice_candidates_handles_out_of_order_duplicates_late_trickle_and_eoc() {
        let mut service = RtcConnectionService::default();
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let session = XbxEngineSessionDto {
            session_id: "test-session".to_string(),
            target_type: XbxEngineTargetTypeDto::Cloud,
            turn_server: None,
        };

        service.rebuild(&session, &runtime_stats).unwrap();
        let offer = service
            .create_raw_offer(&Default::default(), &runtime_stats)
            .unwrap();

        let remote_io = TestRtcPeerIo::bind().unwrap();
        let remote_candidate = remote_io.local_candidate().unwrap();
        let host = candidate_dto(
            &remote_candidate.candidate,
            remote_candidate.sdp_mid.as_deref(),
            remote_candidate.sdp_mline_index,
        );
        let srflx = candidate_dto(
            &remote_candidate.candidate.replace(
                " typ host",
                &format!(
                    " typ srflx raddr {} rport {}",
                    remote_io.local_addr.ip(),
                    remote_io.local_addr.port()
                ),
            ),
            remote_candidate.sdp_mid.as_deref(),
            remote_candidate.sdp_mline_index,
        );
        let relay = candidate_dto(
            &remote_candidate.candidate.replace(
                " typ host",
                &format!(
                    " typ relay raddr {} rport {}",
                    remote_io.local_addr.ip(),
                    remote_io.local_addr.port()
                ),
            ),
            remote_candidate.sdp_mid.as_deref(),
            remote_candidate.sdp_mline_index,
        );

        service
            .add_remote_ice_candidates(&[relay.clone(), host.clone(), host.clone()], &runtime_stats)
            .unwrap();
        service
            .add_remote_ice_candidates(&[candidate_dto("", Some("0"), Some(0))], &runtime_stats)
            .unwrap();

        let mut answer_pc = RTCPeerConnectionBuilder::new()
            .with_configuration(RTCConfigurationBuilder::new().build())
            .build()
            .unwrap();
        answer_pc
            .set_remote_description(RTCSessionDescription::offer(offer).unwrap())
            .unwrap();
        let answer = answer_pc.create_answer(None).unwrap();
        answer_pc.set_local_description(answer.clone()).unwrap();

        service
            .apply_remote_description(
                &answer.sdp,
                &[relay.clone(), srflx.clone(), host.clone(), srflx.clone()],
                &runtime_stats,
            )
            .unwrap();

        let state = service.state.lock().expect("connection state");
        assert!(state.pending_remote_candidates.is_empty());
        assert!(state.pending_remote_candidate_keys.is_empty());
        assert!(state.remote_ice_gathering_complete);
        assert_eq!(state.remote_candidates.len(), 3);
        assert_eq!(state.remote_candidate_keys.len(), 3);
        assert_eq!(state.applied_remote_candidate_keys.len(), 3);
        assert_eq!(state.remote_candidate_host_count, 1);
        assert_eq!(state.remote_candidate_srflx_count, 1);
        assert_eq!(state.remote_candidate_relay_count, 1);
        assert_eq!(state.remote_candidate_unknown_count, 0);
        drop(state);
        assert!(runtime_stats
            .lock()
            .expect("runtime stats")
            .latest_observation_summary
            .as_deref()
            .is_some_and(|summary| summary.contains("remote total=3")));
    }

    #[test]
    fn service_connects_to_raw_rtc_answer_peer_and_opens_control_channel() {
        let mut service = RtcConnectionService::default();
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let session = XbxEngineSessionDto {
            session_id: "test-session".to_string(),
            target_type: XbxEngineTargetTypeDto::Cloud,
            turn_server: None,
        };

        service.rebuild(&session, &runtime_stats).unwrap();
        let offer = service
            .create_raw_offer(&Default::default(), &runtime_stats)
            .unwrap();
        let service_candidates = service.local_candidates_snapshot();
        assert!(
            !service_candidates.is_empty(),
            "service local candidates should not be empty"
        );

        let mut answer_pc = RTCPeerConnectionBuilder::new()
            .with_configuration(RTCConfigurationBuilder::new().build())
            .build()
            .unwrap();
        let mut answer_io = TestRtcPeerIo::bind().unwrap();
        let answer_candidate = answer_io.local_candidate().unwrap();

        answer_pc
            .set_remote_description(RTCSessionDescription::offer(offer).unwrap())
            .unwrap();
        answer_pc
            .add_local_candidate(answer_candidate.clone())
            .unwrap();
        answer_pc
            .add_local_candidate(end_of_candidates_for_test())
            .unwrap();
        for candidate in &service_candidates {
            answer_pc
                .add_remote_candidate(dto_to_rtc_candidate(candidate))
                .unwrap();
        }
        let answer = answer_pc.create_answer(None).unwrap();
        answer_pc.set_local_description(answer.clone()).unwrap();

        service
            .apply_remote_description(
                &answer.sdp,
                &[XbxEngineIceCandidateDto {
                    candidate: answer_candidate.candidate.clone(),
                    sdp_m_line_index: answer_candidate.sdp_mline_index,
                    sdp_mid: answer_candidate.sdp_mid.clone(),
                }],
                &runtime_stats,
            )
            .unwrap();

        let mut answer_connected = false;
        let mut answer_message_dc_id = None;
        let mut answer_control_dc_id = None;
        let mut answer_input_dc_id = None;
        let mut answer_chat_dc_id = None;
        let mut handshake_ack_sent = false;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            service.pump(&runtime_stats).unwrap();
            answer_io.pump(&mut answer_pc).unwrap();
            while let Some(event) = answer_pc.poll_event() {
                match event {
                    RTCPeerConnectionEvent::OnConnectionStateChangeEvent(
                        RTCPeerConnectionState::Connected,
                    ) => {
                        answer_connected = true;
                    }
                    RTCPeerConnectionEvent::OnConnectionStateChangeEvent(
                        RTCPeerConnectionState::Failed,
                    ) => panic!("answer peer connection failed"),
                    RTCPeerConnectionEvent::OnDataChannel(RTCDataChannelEvent::OnOpen(dc_id)) => {
                        let label = answer_pc
                            .data_channel(dc_id)
                            .expect("answer data channel")
                            .label()
                            .to_string();
                        match label.as_str() {
                            MESSAGE_CHANNEL_LABEL => {
                                let _ = answer_message_dc_id.get_or_insert(dc_id);
                            }
                            CONTROL_CHANNEL_LABEL => {
                                let _ = answer_control_dc_id.get_or_insert(dc_id);
                            }
                            INPUT_CHANNEL_LABEL => {
                                let _ = answer_input_dc_id.get_or_insert(dc_id);
                            }
                            CHAT_CHANNEL_LABEL => {
                                let _ = answer_chat_dc_id.get_or_insert(dc_id);
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            while let Some(message) = answer_pc.poll_read() {
                if let rtc::peer_connection::message::RTCMessage::DataChannelMessage(
                    channel_id,
                    payload,
                ) = message
                {
                    let body = String::from_utf8_lossy(payload.data.as_ref()).to_string();
                    if body.contains("\"type\":\"Handshake\"") {
                        let mut answer_dc = answer_pc
                            .data_channel(channel_id)
                            .expect("answer data channel");
                        answer_dc
                            .send_text(HANDSHAKE_ACK_PAYLOAD.to_string())
                            .unwrap();
                        handshake_ack_sent = true;
                    }
                }
            }
            if handshake_ack_sent {
                answer_io.pump(&mut answer_pc).unwrap();
                service.pump(&runtime_stats).unwrap();
                answer_io.pump(&mut answer_pc).unwrap();
                handshake_ack_sent = false;
            }

            if answer_connected
                && answer_message_dc_id.is_some()
                && answer_control_dc_id.is_some()
                && answer_input_dc_id.is_some()
                && answer_chat_dc_id.is_some()
            {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }

        assert!(answer_connected, "answer peer should reach connected");
        assert!(
            answer_message_dc_id.is_some(),
            "message channel should open"
        );
        assert!(
            answer_control_dc_id.is_some(),
            "control channel should open"
        );
        assert!(answer_input_dc_id.is_some(), "input channel should open");
        assert!(answer_chat_dc_id.is_some(), "chat channel should open");
        assert_eq!(
            runtime_stats.lock().unwrap().transport_state,
            xbxengine_protocol::XbxEngineTransportStateDto::Connected
        );
    }

    #[test]
    fn service_pump_observes_data_channel_message_from_poll_read() {
        let mut service = RtcConnectionService::default();
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let session = XbxEngineSessionDto {
            session_id: "test-session".to_string(),
            target_type: XbxEngineTargetTypeDto::Cloud,
            turn_server: None,
        };

        service.rebuild(&session, &runtime_stats).unwrap();
        let offer = service
            .create_raw_offer(&Default::default(), &runtime_stats)
            .unwrap();
        let service_candidates = service.local_candidates_snapshot();
        assert!(
            !service_candidates.is_empty(),
            "service local candidates should not be empty"
        );

        let mut answer_pc = RTCPeerConnectionBuilder::new()
            .with_configuration(RTCConfigurationBuilder::new().build())
            .build()
            .unwrap();
        let mut answer_io = TestRtcPeerIo::bind().unwrap();
        let answer_candidate = answer_io.local_candidate().unwrap();

        answer_pc
            .set_remote_description(RTCSessionDescription::offer(offer).unwrap())
            .unwrap();
        answer_pc
            .add_local_candidate(answer_candidate.clone())
            .unwrap();
        answer_pc
            .add_local_candidate(end_of_candidates_for_test())
            .unwrap();
        for candidate in &service_candidates {
            answer_pc
                .add_remote_candidate(dto_to_rtc_candidate(candidate))
                .unwrap();
        }
        let answer = answer_pc.create_answer(None).unwrap();
        answer_pc.set_local_description(answer.clone()).unwrap();

        service
            .apply_remote_description(
                &answer.sdp,
                &[XbxEngineIceCandidateDto {
                    candidate: answer_candidate.candidate.clone(),
                    sdp_m_line_index: answer_candidate.sdp_mline_index,
                    sdp_mid: answer_candidate.sdp_mid.clone(),
                }],
                &runtime_stats,
            )
            .unwrap();

        let mut answer_connected = false;
        let mut answer_message_dc_id = None;
        let mut answer_control_dc_id = None;
        let mut answer_input_dc_id = None;
        let mut answer_chat_dc_id = None;
        let mut handshake_ack_sent = false;
        let mut saw_input_metadata = false;
        let connect_deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < connect_deadline {
            service.pump(&runtime_stats).unwrap();
            answer_io.pump(&mut answer_pc).unwrap();
            while let Some(event) = answer_pc.poll_event() {
                match event {
                    RTCPeerConnectionEvent::OnConnectionStateChangeEvent(
                        RTCPeerConnectionState::Connected,
                    ) => {
                        answer_connected = true;
                    }
                    RTCPeerConnectionEvent::OnDataChannel(RTCDataChannelEvent::OnOpen(dc_id)) => {
                        let label = answer_pc
                            .data_channel(dc_id)
                            .expect("answer data channel")
                            .label()
                            .to_string();
                        match label.as_str() {
                            MESSAGE_CHANNEL_LABEL => {
                                let _ = answer_message_dc_id.get_or_insert(dc_id);
                            }
                            CONTROL_CHANNEL_LABEL => {
                                let _ = answer_control_dc_id.get_or_insert(dc_id);
                            }
                            INPUT_CHANNEL_LABEL => {
                                let _ = answer_input_dc_id.get_or_insert(dc_id);
                            }
                            CHAT_CHANNEL_LABEL => {
                                let _ = answer_chat_dc_id.get_or_insert(dc_id);
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            while let Some(message) = answer_pc.poll_read() {
                if let rtc::peer_connection::message::RTCMessage::DataChannelMessage(
                    channel_id,
                    payload,
                ) = message
                {
                    let label = answer_pc
                        .data_channel(channel_id)
                        .expect("answer data channel")
                        .label()
                        .to_string();
                    let body = String::from_utf8_lossy(payload.data.as_ref()).to_string();
                    if label == INPUT_CHANNEL_LABEL && !payload.is_string {
                        let bytes = payload.data.as_ref();
                        saw_input_metadata =
                            bytes.len() == 15 && u16::from_le_bytes([bytes[0], bytes[1]]) == 8;
                    }
                    if label == MESSAGE_CHANNEL_LABEL && body.contains("\"type\":\"Handshake\"") {
                        let mut answer_dc = answer_pc
                            .data_channel(channel_id)
                            .expect("answer message channel");
                        answer_dc
                            .send_text(HANDSHAKE_ACK_PAYLOAD.to_string())
                            .unwrap();
                        handshake_ack_sent = true;
                    }
                }
            }
            if handshake_ack_sent {
                service.pump(&runtime_stats).unwrap();
                answer_io.pump(&mut answer_pc).unwrap();
                handshake_ack_sent = false;
            }
            if answer_connected
                && service.control_service.is_control_ready()
                && answer_message_dc_id.is_some()
                && answer_control_dc_id.is_some()
                && answer_input_dc_id.is_some()
                && answer_chat_dc_id.is_some()
            {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }

        assert!(answer_connected, "answer peer should reach connected");
        assert!(
            answer_message_dc_id.is_some(),
            "message channel should open"
        );
        assert!(
            answer_control_dc_id.is_some(),
            "control channel should open"
        );
        assert!(answer_input_dc_id.is_some(), "input channel should open");
        assert!(answer_chat_dc_id.is_some(), "chat channel should open");
        assert!(
            service.control_service.is_control_ready(),
            "service control channel should become ready"
        );

        let input_dc_id = answer_input_dc_id.expect("answer input channel id");
        let input_deadline = Instant::now() + Duration::from_secs(3);
        let mut saw_input_metadata = saw_input_metadata;
        while Instant::now() < input_deadline {
            service.pump(&runtime_stats).unwrap();
            answer_io.pump(&mut answer_pc).unwrap();
            while let Some(message) = answer_pc.poll_read() {
                if let rtc::peer_connection::message::RTCMessage::DataChannelMessage(
                    channel_id,
                    payload,
                ) = message
                {
                    let label = answer_pc
                        .data_channel(channel_id)
                        .expect("answer data channel")
                        .label()
                        .to_string();
                    if label == INPUT_CHANNEL_LABEL && !payload.is_string {
                        let bytes = payload.data.as_ref();
                        saw_input_metadata =
                            bytes.len() == 15 && u16::from_le_bytes([bytes[0], bytes[1]]) == 8;
                    }
                }
            }
            if saw_input_metadata {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            saw_input_metadata,
            "service should send input metadata bootstrap"
        );

        let chat_dc_id = answer_chat_dc_id.expect("answer chat channel id");
        let chat_payload = "hello from rtc chat";
        {
            let mut answer_chat_dc = answer_pc
                .data_channel(chat_dc_id)
                .expect("answer chat channel available");
            answer_chat_dc.send_text(chat_payload.to_string()).unwrap();
        }

        let chat_deadline = Instant::now() + Duration::from_secs(3);
        let mut saw_chat_catalog = false;
        while Instant::now() < chat_deadline {
            service.pump(&runtime_stats).unwrap();
            answer_io.pump(&mut answer_pc).unwrap();
            if let Ok(stats) = runtime_stats.lock() {
                if let Some(observation) = stats
                    .latest_data_channel_message_catalog_observation
                    .as_ref()
                {
                    saw_chat_catalog = observation.channel == "chat"
                        && observation.kind_message.as_deref() == Some("text")
                        && observation.payload_len == chat_payload.len();
                    if saw_chat_catalog
                        && stats.latest_observation_label.as_deref() == Some("rtcChatTextObserved")
                    {
                        break;
                    }
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(saw_chat_catalog, "service should catalog inbound chat text");

        {
            let mut answer_chat_dc = answer_pc
                .data_channel(chat_dc_id)
                .expect("answer chat channel available");
            answer_chat_dc.close().unwrap();
        }
        let chat_close_deadline = Instant::now() + Duration::from_secs(3);
        let mut saw_chat_closed = false;
        while Instant::now() < chat_close_deadline {
            service.pump(&runtime_stats).unwrap();
            answer_io.pump(&mut answer_pc).unwrap();
            if runtime_stats.lock().ok().is_some_and(|stats| {
                stats.latest_observation_label.as_deref() == Some("rtcChatChannelClosed")
            }) {
                saw_chat_closed = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(saw_chat_closed, "service should observe chat close");

        {
            let mut answer_input_dc = answer_pc
                .data_channel(input_dc_id)
                .expect("answer input channel available");
            answer_input_dc.close().unwrap();
        }
        let input_close_deadline = Instant::now() + Duration::from_secs(3);
        let mut saw_input_closed = false;
        while Instant::now() < input_close_deadline {
            service.pump(&runtime_stats).unwrap();
            answer_io.pump(&mut answer_pc).unwrap();
            if service
                .state
                .lock()
                .ok()
                .is_some_and(|state| !state.input_channel_open)
                && runtime_stats.lock().ok().is_some_and(|stats| {
                    stats.latest_observation_label.as_deref() == Some("rtcInputChannelClosed")
                })
            {
                saw_input_closed = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(saw_input_closed, "service should observe input close");

        assert!(service.request_video_keyframe(&runtime_stats).is_ok());
    }

    #[test]
    fn service_bootstraps_message_and_control_payloads_after_handshake_ack() {
        let mut service = RtcConnectionService::default();
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let session = XbxEngineSessionDto {
            session_id: "test-session".to_string(),
            target_type: XbxEngineTargetTypeDto::Cloud,
            turn_server: None,
        };

        service.rebuild(&session, &runtime_stats).unwrap();
        let offer = service
            .create_raw_offer(&Default::default(), &runtime_stats)
            .unwrap();
        let service_candidates = service.local_candidates_snapshot();
        assert!(
            !service_candidates.is_empty(),
            "service local candidates should not be empty"
        );

        let mut answer_pc = RTCPeerConnectionBuilder::new()
            .with_configuration(RTCConfigurationBuilder::new().build())
            .build()
            .unwrap();
        let mut answer_io = TestRtcPeerIo::bind().unwrap();
        let answer_candidate = answer_io.local_candidate().unwrap();

        answer_pc
            .set_remote_description(RTCSessionDescription::offer(offer).unwrap())
            .unwrap();
        answer_pc
            .add_local_candidate(answer_candidate.clone())
            .unwrap();
        answer_pc
            .add_local_candidate(end_of_candidates_for_test())
            .unwrap();
        for candidate in &service_candidates {
            answer_pc
                .add_remote_candidate(dto_to_rtc_candidate(candidate))
                .unwrap();
        }
        let answer = answer_pc.create_answer(None).unwrap();
        answer_pc.set_local_description(answer.clone()).unwrap();

        service
            .apply_remote_description(
                &answer.sdp,
                &[XbxEngineIceCandidateDto {
                    candidate: answer_candidate.candidate.clone(),
                    sdp_m_line_index: answer_candidate.sdp_mline_index,
                    sdp_mid: answer_candidate.sdp_mid.clone(),
                }],
                &runtime_stats,
            )
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut answer_connected = false;
        let mut answer_message_dc_id = None;
        let mut answer_control_dc_id = None;
        let mut answer_input_dc_id = None;
        let mut answer_chat_dc_id = None;
        let mut saw_post_handshake = false;
        let mut saw_control_authorization = false;
        let mut saw_control_removed = false;
        let mut saw_keyframe_request = false;
        let mut saw_input_metadata = false;
        let mut saw_chat_catalog = false;
        let mut observed_message_payloads = Vec::new();

        while Instant::now() < deadline {
            service.pump(&runtime_stats).unwrap();
            answer_io.pump(&mut answer_pc).unwrap();
            while let Some(event) = answer_pc.poll_event() {
                match event {
                    RTCPeerConnectionEvent::OnConnectionStateChangeEvent(
                        RTCPeerConnectionState::Connected,
                    ) => {
                        answer_connected = true;
                    }
                    RTCPeerConnectionEvent::OnDataChannel(RTCDataChannelEvent::OnOpen(dc_id)) => {
                        let label = answer_pc
                            .data_channel(dc_id)
                            .expect("answer data channel")
                            .label()
                            .to_string();
                        match label.as_str() {
                            MESSAGE_CHANNEL_LABEL => {
                                let _ = answer_message_dc_id.get_or_insert(dc_id);
                            }
                            CONTROL_CHANNEL_LABEL => {
                                let _ = answer_control_dc_id.get_or_insert(dc_id);
                            }
                            INPUT_CHANNEL_LABEL => {
                                let _ = answer_input_dc_id.get_or_insert(dc_id);
                            }
                            CHAT_CHANNEL_LABEL => {
                                let _ = answer_chat_dc_id.get_or_insert(dc_id);
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            while let Some(message) = answer_pc.poll_read() {
                if let rtc::peer_connection::message::RTCMessage::DataChannelMessage(
                    channel_id,
                    payload,
                ) = message
                {
                    let label = answer_pc
                        .data_channel(channel_id)
                        .expect("answer data channel")
                        .label()
                        .to_string();
                    let body = String::from_utf8_lossy(payload.data.as_ref()).to_string();
                    if label == MESSAGE_CHANNEL_LABEL {
                        observed_message_payloads.push(body.clone());
                    }
                    if label == MESSAGE_CHANNEL_LABEL && body.contains("\"type\":\"Handshake\"") {
                        let mut answer_dc = answer_pc
                            .data_channel(channel_id)
                            .expect("answer message channel");
                        answer_dc
                            .send_text(HANDSHAKE_ACK_PAYLOAD.to_string())
                            .unwrap();
                    }
                    if label == INPUT_CHANNEL_LABEL && !payload.is_string {
                        let bytes = payload.data.as_ref();
                        saw_input_metadata =
                            bytes.len() == 15 && u16::from_le_bytes([bytes[0], bytes[1]]) == 8;
                    }
                    if label == CHAT_CHANNEL_LABEL && payload.is_string {
                        saw_chat_catalog = body.contains("hello from rtc chat");
                    }
                    if label == CONTROL_CHANNEL_LABEL
                        && body.contains("\"message\":\"videoKeyframeRequested\"")
                    {
                        saw_keyframe_request = true;
                    }
                    if body.contains("/streaming/systemUi/configuration")
                        || body.contains("/streaming/properties/clientappinstallidchanged")
                    {
                        saw_post_handshake = true;
                    }
                    if body.contains("\"message\":\"authorizationRequest\"") {
                        saw_control_authorization = true;
                    }
                    if body.contains("\"message\":\"gamepadChanged\"")
                        && body.contains("\"wasAdded\":false")
                    {
                        saw_control_removed = true;
                    }
                }
            }

            if service.control_service.is_control_ready() && !saw_keyframe_request {
                service.request_video_keyframe(&runtime_stats).unwrap();
            }

            if answer_connected
                && service.control_service.is_control_ready()
                && answer_message_dc_id.is_some()
                && answer_control_dc_id.is_some()
                && answer_input_dc_id.is_some()
                && answer_chat_dc_id.is_some()
                && saw_post_handshake
                && saw_control_authorization
                && saw_control_removed
                && saw_keyframe_request
            {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }

        assert!(answer_connected, "answer peer should reach connected");
        assert!(
            answer_message_dc_id.is_some(),
            "message channel should open"
        );
        assert!(
            answer_control_dc_id.is_some(),
            "control channel should open"
        );
        assert!(answer_input_dc_id.is_some(), "input channel should open");
        assert!(answer_chat_dc_id.is_some(), "chat channel should open");
        assert!(
            saw_post_handshake,
            "service should send post-handshake message payload, observed message payloads: {observed_message_payloads:?}"
        );
        assert!(
            saw_control_authorization,
            "service should send control authorization payload"
        );
        assert!(
            saw_control_removed,
            "service should send control gamepad removed payload"
        );
        assert!(
            saw_keyframe_request,
            "service should send keyframe request after control becomes ready"
        );

        let chat_payload = "hello from rtc chat";
        {
            let mut answer_chat_dc = answer_pc
                .data_channel(answer_chat_dc_id.expect("answer chat channel id"))
                .expect("answer chat channel available");
            answer_chat_dc.send_text(chat_payload.to_string()).unwrap();
        }

        let chat_deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < chat_deadline {
            service.pump(&runtime_stats).unwrap();
            answer_io.pump(&mut answer_pc).unwrap();
            while let Some(message) = answer_pc.poll_read() {
                if let rtc::peer_connection::message::RTCMessage::DataChannelMessage(
                    channel_id,
                    payload,
                ) = message
                {
                    let label = answer_pc
                        .data_channel(channel_id)
                        .expect("answer data channel")
                        .label()
                        .to_string();
                    if label == INPUT_CHANNEL_LABEL && !payload.is_string {
                        let bytes = payload.data.as_ref();
                        saw_input_metadata =
                            bytes.len() == 15 && u16::from_le_bytes([bytes[0], bytes[1]]) == 8;
                    }
                }
            }
            if let Ok(stats) = runtime_stats.lock() {
                if let Some(observation) = stats
                    .latest_data_channel_message_catalog_observation
                    .as_ref()
                {
                    saw_chat_catalog = observation.channel == "chat"
                        && observation.kind_message.as_deref() == Some("text")
                        && observation.payload_len == chat_payload.len();
                }
            }
            if saw_input_metadata && saw_chat_catalog {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            saw_input_metadata,
            "service should send input metadata bootstrap"
        );
        assert!(saw_chat_catalog, "service should catalog inbound chat text");

        assert!(service.request_video_keyframe(&runtime_stats).is_ok());
    }

    #[test]
    fn service_pump_error_marks_recovering_and_exposes_pending_reconnect_action() {
        let mut service = RtcConnectionService::default();
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let session = XbxEngineSessionDto {
            session_id: "test-session".to_string(),
            target_type: XbxEngineTargetTypeDto::Cloud,
            turn_server: None,
        };

        service.rebuild(&session, &runtime_stats).unwrap();
        let _ = service
            .create_raw_offer(&Default::default(), &runtime_stats)
            .unwrap();

        service.inject_pump_failure();

        let error = service
            .pump(&runtime_stats)
            .expect_err("pump should fail after local address fault injection");
        assert!(
            error
                .to_string()
                .contains("xbxEngineRtcPumpInjectedFailure"),
            "unexpected pump error: {error}"
        );
        assert!(
            matches!(
                service.lifecycle_state,
                RtcConnectionLifecycleState::Failed | RtcConnectionLifecycleState::Recovering
            ),
            "service should enter failed/recovering lifecycle"
        );
        assert!(
            service.take_pending_runtime_recovery_action().is_some(),
            "pump error should expose pending reconnect action"
        );
        assert!(matches!(
            runtime_stats.lock().unwrap().transport_state,
            xbxengine_protocol::XbxEngineTransportStateDto::Failed
                | xbxengine_protocol::XbxEngineTransportStateDto::Connecting
        ));
    }

    #[test]
    fn service_rebuild_preserves_remote_candidate_cache_for_reconnect() {
        let mut service = RtcConnectionService::default();
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let session = XbxEngineSessionDto {
            session_id: "test-session".to_string(),
            target_type: XbxEngineTargetTypeDto::Cloud,
            turn_server: None,
        };

        service.rebuild(&session, &runtime_stats).unwrap();
        let duplicate = service
            .local_candidates_snapshot()
            .into_iter()
            .next()
            .expect("service local candidate");
        service
            .add_remote_ice_candidates(&[duplicate.clone(), duplicate], &runtime_stats)
            .unwrap();

        let state_before = service.state.lock().expect("connection state").clone();
        service.rebuild(&session, &runtime_stats).unwrap();
        let state_after = service.state.lock().expect("connection state");

        assert_eq!(
            state_after.remote_candidates.len(),
            state_before.remote_candidates.len(),
            "remote candidates should survive reconnect rebuild"
        );
        assert_eq!(
            state_after.pending_remote_candidates.len(),
            state_before.pending_remote_candidates.len(),
            "pending remote candidates should survive reconnect rebuild"
        );
        assert_eq!(
            state_after.remote_candidate_keys.len(),
            state_before.remote_candidate_keys.len(),
            "remote candidate keys should survive reconnect rebuild"
        );
        assert_eq!(
            state_after.pending_remote_candidate_keys.len(),
            state_before.pending_remote_candidate_keys.len(),
            "pending remote candidate keys should survive reconnect rebuild"
        );
    }

    #[test]
    fn service_replays_pending_control_requests_after_control_close_and_rebuild() {
        let mut service = RtcConnectionService::default();
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let session = XbxEngineSessionDto {
            session_id: "test-session".to_string(),
            target_type: XbxEngineTargetTypeDto::Cloud,
            turn_server: None,
        };

        service.rebuild(&session, &runtime_stats).unwrap();
        let (
            mut answer_pc,
            mut answer_io,
            _message_dc_id,
            control_dc_id,
            _input_dc_id,
            _chat_dc_id,
            _saw_input_metadata,
            observed_payloads,
        ) = connect_service_to_answer_peer(&mut service, &runtime_stats);
        let control_dc_id = control_dc_id.expect("answer control channel id");
        assert!(
            service.control_service.is_control_ready(),
            "service should be control-ready before injecting failure"
        );
        assert!(
            observed_payloads
                .iter()
                .any(|(label, body)| label == MESSAGE_CHANNEL_LABEL && body.contains("Handshake")),
            "handshake should have been observed before failure injection"
        );

        {
            let mut answer_control_dc = answer_pc
                .data_channel(control_dc_id)
                .expect("answer control channel available");
            answer_control_dc.close().unwrap();
        }
        let close_deadline = Instant::now() + Duration::from_secs(3);
        let mut saw_control_closed = false;
        while Instant::now() < close_deadline {
            service.pump(&runtime_stats).unwrap();
            answer_io.pump(&mut answer_pc).unwrap();
            if runtime_stats.lock().ok().is_some_and(|stats| {
                stats.latest_observation_label.as_deref() == Some("rtcControlChannelClosed")
            }) {
                saw_control_closed = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(saw_control_closed, "service should observe control close");

        assert!(service.request_video_keyframe(&runtime_stats).is_err());
        assert!(service.request_decoder_reset(&runtime_stats).is_err());
        assert!(
            service.control_service.has_pending_replay_actions(),
            "pending replay requests should be retained while reconnecting"
        );

        drop(answer_pc);
        drop(answer_io);
        service.rebuild(&session, &runtime_stats).unwrap();

        let (
            _reconnect_pc,
            _reconnect_io,
            reconnect_message_dc_id,
            reconnect_control_dc_id,
            reconnect_input_dc_id,
            reconnect_chat_dc_id,
            _saw_input_metadata,
            reconnect_payloads,
        ) = connect_service_to_answer_peer(&mut service, &runtime_stats);

        assert!(
            reconnect_message_dc_id.is_some(),
            "message channel should reopen"
        );
        assert!(
            reconnect_control_dc_id.is_some(),
            "control channel should reopen"
        );
        assert!(
            reconnect_input_dc_id.is_some(),
            "input channel should reopen"
        );
        assert!(reconnect_chat_dc_id.is_some(), "chat channel should reopen");
        assert!(
            service.control_service.is_control_ready(),
            "service should become control-ready again after reconnect"
        );

        let replay_keyframe = reconnect_payloads.iter().any(|(label, body)| {
            label == CONTROL_CHANNEL_LABEL
                && body.contains("\"message\":\"videoKeyframeRequested\"")
        });
        let replay_decoder_reset = reconnect_payloads.iter().any(|(label, body)| {
            label == CONTROL_CHANNEL_LABEL && body.contains("\"message\":\"decoderReset\"")
        });
        assert!(
            replay_keyframe,
            "reconnect should replay pending keyframe request"
        );
        assert!(
            replay_decoder_reset,
            "reconnect should replay pending decoder reset request"
        );
    }

    #[derive(Debug)]
    struct TestRtcPeerIo {
        socket: UdpSocket,
        local_addr: SocketAddr,
    }

    impl TestRtcPeerIo {
        fn bind() -> Result<Self, XbxEngineRuntimeError> {
            let socket = UdpSocket::bind("127.0.0.1:0").map_err(|err| {
                XbxEngineRuntimeError::new(format!("xbxEngineRtcTestIoBindFailed: {err}"))
            })?;
            socket.set_nonblocking(true).map_err(|err| {
                XbxEngineRuntimeError::new(format!("xbxEngineRtcTestIoSetNonblockingFailed: {err}"))
            })?;
            let local_addr = socket.local_addr().map_err(|err| {
                XbxEngineRuntimeError::new(format!("xbxEngineRtcTestIoLocalAddrFailed: {err}"))
            })?;
            Ok(Self { socket, local_addr })
        }

        fn local_candidate(&self) -> Result<RTCIceCandidateInit, XbxEngineRuntimeError> {
            let candidate = CandidateHostConfig {
                base_config: CandidateConfig {
                    network: "udp".to_string(),
                    address: self.local_addr.ip().to_string(),
                    port: self.local_addr.port(),
                    component: 1,
                    ..Default::default()
                },
                ..Default::default()
            }
            .new_candidate_host()
            .map_err(|err| {
                XbxEngineRuntimeError::new(format!("xbxEngineRtcTestHostCandidateFailed: {err}"))
            })?;
            let mut candidate_init =
                RTCIceCandidate::from(&candidate).to_json().map_err(|err| {
                    XbxEngineRuntimeError::new(format!(
                        "xbxEngineRtcTestCandidateToJsonFailed: {err}"
                    ))
                })?;
            candidate_init.sdp_mid = Some("0".to_string());
            candidate_init.sdp_mline_index = Some(0);
            Ok(candidate_init)
        }

        fn pump(
            &mut self,
            peer_connection: &mut rtc::peer_connection::RTCPeerConnection,
        ) -> Result<(), XbxEngineRuntimeError> {
            let mut buffer = [0u8; 2_048];

            for _ in 0..8 {
                let mut progressed = false;

                while let Some(deadline) = peer_connection.poll_timeout() {
                    let now = Instant::now();
                    if deadline > now {
                        break;
                    }
                    peer_connection.handle_timeout(now).map_err(|err| {
                        XbxEngineRuntimeError::new(format!(
                            "xbxEngineRtcTestHandleTimeoutFailed: {err}"
                        ))
                    })?;
                    progressed = true;
                }

                while let Some(message) = peer_connection.poll_write() {
                    match self
                        .socket
                        .send_to(&message.message, message.transport.peer_addr)
                    {
                        Ok(_) => progressed = true,
                        Err(err) if err.kind() == ErrorKind::WouldBlock => break,
                        Err(err) => {
                            return Err(XbxEngineRuntimeError::new(format!(
                                "xbxEngineRtcTestSocketSendFailed: {err}"
                            )));
                        }
                    }
                }

                loop {
                    match self.socket.recv_from(&mut buffer) {
                        Ok((size, peer_addr)) => {
                            peer_connection
                                .handle_read(TaggedBytesMut {
                                    now: Instant::now(),
                                    transport: TransportContext {
                                        local_addr: self.local_addr,
                                        peer_addr,
                                        transport_protocol: TransportProtocol::UDP,
                                        ecn: None,
                                    },
                                    message: BytesMut::from(&buffer[..size]),
                                })
                                .map_err(|err| {
                                    XbxEngineRuntimeError::new(format!(
                                        "xbxEngineRtcTestHandleReadFailed: {err}"
                                    ))
                                })?;
                            progressed = true;
                        }
                        Err(err) if err.kind() == ErrorKind::WouldBlock => break,
                        Err(err) => {
                            return Err(XbxEngineRuntimeError::new(format!(
                                "xbxEngineRtcTestSocketReadFailed: {err}"
                            )));
                        }
                    }
                }

                if !progressed {
                    break;
                }
            }

            Ok(())
        }
    }

    fn end_of_candidates_for_test() -> RTCIceCandidateInit {
        RTCIceCandidateInit {
            candidate: String::new(),
            sdp_mid: Some("0".to_string()),
            sdp_mline_index: Some(0),
            username_fragment: None,
            url: None,
        }
    }

    fn candidate_dto(
        candidate: &str,
        sdp_mid: Option<&str>,
        sdp_m_line_index: Option<u16>,
    ) -> XbxEngineIceCandidateDto {
        XbxEngineIceCandidateDto {
            candidate: candidate.to_string(),
            sdp_m_line_index,
            sdp_mid: sdp_mid.map(|value| value.to_string()),
        }
    }

    fn connect_service_to_answer_peer(
        service: &mut RtcConnectionService,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> (
        RTCPeerConnection,
        TestRtcPeerIo,
        Option<u16>,
        Option<u16>,
        Option<u16>,
        Option<u16>,
        bool,
        Vec<(String, String)>,
    ) {
        let offer = service
            .create_raw_offer(&Default::default(), runtime_stats)
            .unwrap();
        let service_candidates = service.local_candidates_snapshot();
        assert!(
            !service_candidates.is_empty(),
            "service local candidates should not be empty"
        );

        let mut answer_pc = RTCPeerConnectionBuilder::new()
            .with_configuration(RTCConfigurationBuilder::new().build())
            .build()
            .unwrap();
        let mut answer_io = TestRtcPeerIo::bind().unwrap();
        let answer_candidate = answer_io.local_candidate().unwrap();

        answer_pc
            .set_remote_description(RTCSessionDescription::offer(offer).unwrap())
            .unwrap();
        answer_pc
            .add_local_candidate(answer_candidate.clone())
            .unwrap();
        answer_pc
            .add_local_candidate(end_of_candidates_for_test())
            .unwrap();
        for candidate in &service_candidates {
            answer_pc
                .add_remote_candidate(dto_to_rtc_candidate(candidate))
                .unwrap();
        }
        let answer = answer_pc.create_answer(None).unwrap();
        answer_pc.set_local_description(answer.clone()).unwrap();

        service
            .apply_remote_description(
                &answer.sdp,
                &[XbxEngineIceCandidateDto {
                    candidate: answer_candidate.candidate.clone(),
                    sdp_m_line_index: answer_candidate.sdp_mline_index,
                    sdp_mid: answer_candidate.sdp_mid.clone(),
                }],
                runtime_stats,
            )
            .unwrap();

        let mut answer_connected = false;
        let mut answer_message_dc_id = None;
        let mut answer_control_dc_id = None;
        let mut answer_input_dc_id = None;
        let mut answer_chat_dc_id = None;
        let mut observed_payloads = Vec::new();
        let mut handshake_ack_sent = false;
        let mut saw_input_metadata = false;
        let mut ready_streak: u8 = 0;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            service.pump(runtime_stats).unwrap();
            answer_io.pump(&mut answer_pc).unwrap();
            while let Some(event) = answer_pc.poll_event() {
                match event {
                    RTCPeerConnectionEvent::OnConnectionStateChangeEvent(
                        RTCPeerConnectionState::Connected,
                    ) => {
                        answer_connected = true;
                    }
                    RTCPeerConnectionEvent::OnConnectionStateChangeEvent(
                        RTCPeerConnectionState::Failed,
                    ) => panic!("answer peer connection failed"),
                    RTCPeerConnectionEvent::OnDataChannel(RTCDataChannelEvent::OnOpen(dc_id)) => {
                        let label = answer_pc
                            .data_channel(dc_id)
                            .expect("answer data channel")
                            .label()
                            .to_string();
                        match label.as_str() {
                            MESSAGE_CHANNEL_LABEL => {
                                let _ = answer_message_dc_id.get_or_insert(dc_id);
                            }
                            CONTROL_CHANNEL_LABEL => {
                                let _ = answer_control_dc_id.get_or_insert(dc_id);
                            }
                            INPUT_CHANNEL_LABEL => {
                                let _ = answer_input_dc_id.get_or_insert(dc_id);
                            }
                            CHAT_CHANNEL_LABEL => {
                                let _ = answer_chat_dc_id.get_or_insert(dc_id);
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            while let Some(message) = answer_pc.poll_read() {
                if let rtc::peer_connection::message::RTCMessage::DataChannelMessage(
                    channel_id,
                    payload,
                ) = message
                {
                    let label = answer_pc
                        .data_channel(channel_id)
                        .expect("answer data channel")
                        .label()
                        .to_string();
                    let body = String::from_utf8_lossy(payload.data.as_ref()).to_string();
                    observed_payloads.push((label.clone(), body.clone()));
                    if label == INPUT_CHANNEL_LABEL && !payload.is_string {
                        let bytes = payload.data.as_ref();
                        saw_input_metadata =
                            bytes.len() == 15 && u16::from_le_bytes([bytes[0], bytes[1]]) == 8;
                    }
                    if label == MESSAGE_CHANNEL_LABEL && body.contains("\"type\":\"Handshake\"") {
                        let mut answer_dc = answer_pc
                            .data_channel(channel_id)
                            .expect("answer message channel");
                        answer_dc
                            .send_text(HANDSHAKE_ACK_PAYLOAD.to_string())
                            .unwrap();
                        handshake_ack_sent = true;
                    }
                }
            }

            if handshake_ack_sent {
                answer_io.pump(&mut answer_pc).unwrap();
                service.pump(runtime_stats).unwrap();
                answer_io.pump(&mut answer_pc).unwrap();
                handshake_ack_sent = false;
            }

            let ready = answer_connected
                && service.control_service.is_control_ready()
                && answer_message_dc_id.is_some()
                && answer_control_dc_id.is_some()
                && answer_input_dc_id.is_some()
                && answer_chat_dc_id.is_some();
            if ready {
                ready_streak = ready_streak.saturating_add(1);
            } else {
                ready_streak = 0;
            }
            if ready_streak >= 2 {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }

        assert!(answer_connected, "answer peer should reach connected");
        assert!(
            answer_message_dc_id.is_some(),
            "message channel should open"
        );
        assert!(
            answer_control_dc_id.is_some(),
            "control channel should open"
        );
        assert!(answer_input_dc_id.is_some(), "input channel should open");
        assert!(answer_chat_dc_id.is_some(), "chat channel should open");
        let control_state = service.control_service.state().clone();
        let latest_observation_label = runtime_stats
            .lock()
            .ok()
            .and_then(|stats| stats.latest_observation_label.clone());
        assert!(
            service.control_service.is_control_ready(),
            "service control channel should become ready, observed payloads: {observed_payloads:?}, state: message_open={} message_acked={} post_handshake_sent={} control_open={} control_started={} control_bootstrapped_after_handshake={} pending_keyframe={} pending_decoder_reset={} lifecycle={:?} latest_observation={latest_observation_label:?}",
            control_state.message_channel_open,
            control_state.message_handshake_acked,
            control_state.post_handshake_messages_sent,
            control_state.control_channel_open,
            control_state.control_started,
            control_state.control_bootstrapped_after_handshake,
            control_state.pending_keyframe_request,
            control_state.pending_decoder_reset,
            service.lifecycle_state
        );

        let replay_deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < replay_deadline {
            service.pump(runtime_stats).unwrap();
            answer_io.pump(&mut answer_pc).unwrap();
            while let Some(message) = answer_pc.poll_read() {
                if let rtc::peer_connection::message::RTCMessage::DataChannelMessage(
                    channel_id,
                    payload,
                ) = message
                {
                    let label = answer_pc
                        .data_channel(channel_id)
                        .expect("answer data channel")
                        .label()
                        .to_string();
                    let body = String::from_utf8_lossy(payload.data.as_ref()).to_string();
                    observed_payloads.push((label, body));
                }
            }
            let replay_keyframe = observed_payloads.iter().any(|(label, body)| {
                label == CONTROL_CHANNEL_LABEL
                    && body.contains("\"message\":\"videoKeyframeRequested\"")
            });
            let replay_decoder_reset = observed_payloads.iter().any(|(label, body)| {
                label == CONTROL_CHANNEL_LABEL && body.contains("\"message\":\"decoderReset\"")
            });
            if replay_keyframe && replay_decoder_reset {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }

        (
            answer_pc,
            answer_io,
            answer_message_dc_id,
            answer_control_dc_id,
            answer_input_dc_id,
            answer_chat_dc_id,
            saw_input_metadata,
            observed_payloads,
        )
    }
}
