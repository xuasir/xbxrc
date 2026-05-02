use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rtc::interceptor::{
    Interceptor, NoopInterceptor, Packet, RTCPFeedback, RTPHeaderExtension, Registry, StreamInfo,
    TaggedPacket, TwccReceiverBuilder, TwccReceiverInterceptor,
};
use rtc::media_stream::MediaStreamTrackId;
use rtc::rtcp::transport_feedbacks::transport_layer_cc::TransportLayerCc;
use rtc::rtp_transceiver::RTCRtpReceiverId;
use rtc::sansio::Protocol;
use rtc::shared::marshal::{MarshalSize, Unmarshal};
use rtc::shared::TransportContext;
use rtc_rtp::extension::transport_cc_extension::TransportCcExtension;

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::connection::builder::ControlledPeerConnection;
use crate::transport::rtc::connection::transport_metrics::{
    build_twcc_observation_with_packet_bytes, TWCC_OBSERVATION_SOURCE_LOCAL_FEEDBACK,
};
use crate::transport::rtc::stats::now_ms_f64;
use crate::{
    XbxEngineMediaRuntimeStats, XbxEngineRuntimeError, XbxEngineTwccExtensionObservation,
    XbxEngineTwccRemoteStreamObservation,
};

const TRANSPORT_CC_URI: &str =
    "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01";
const TWCC_PACKET_MISS_LOG_INTERVAL: u64 = 512;
const TWCC_PENDING_FEEDBACK_MAX: usize = 128;
const TWCC_PACKET_BYTES_LEDGER_WINDOW_MS: f64 = 4_000.0;
pub(super) const VIDEO_TWCC_FEEDBACK_TARGET: &str = "videoTwccFeedback";
static TWCC_REMOTE_STREAM_OBSERVATION_ID: AtomicU64 = AtomicU64::new(0);
static TWCC_EXTENSION_OBSERVATION_ID: AtomicU64 = AtomicU64::new(0);

type LocalTwccInterceptor = TwccReceiverInterceptor<NoopInterceptor>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct TwccSdpBindingInfo {
    pub(super) twcc_ext_id: Option<u8>,
    pub(super) header_extensions: Vec<String>,
    pub(super) rtcp_feedback: Vec<String>,
    pub(super) mime_type: Option<String>,
}

#[derive(Clone, Debug)]
struct ControlledTwccStreamBinding {
    receiver_id: Option<RTCRtpReceiverId>,
    track_id: String,
    mime_type: String,
    payload_type: u8,
    rtcp_feedback: Vec<String>,
    twcc_ext_id: u8,
    packet_seen_count: u64,
    missing_extension_count: u64,
}

#[derive(Clone, Debug)]
struct VideoFeedbackBootstrapBinding {
    ssrc: u32,
    track_id: String,
    mime_type: String,
    payload_type: u8,
    rtcp_feedback: Vec<String>,
    twcc_ext_id: u8,
}

struct PendingTwccFeedbackPacket {
    media_ssrc: Option<u32>,
    packet: Box<dyn rtc::rtcp::Packet>,
}

#[derive(Clone, Debug)]
struct TwccPacketBytesSample {
    observed_at_ms: f64,
    transport_sequence: u16,
    packet_bytes: u32,
}

pub(super) fn record_twcc_remote_stream_observation(
    runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ssrc: u32,
    mime_type: String,
    twcc_ext_id: Option<u8>,
    header_extensions: Vec<String>,
    rtcp_feedback: Vec<String>,
) {
    RuntimeStatsSink::new(runtime_stats.clone()).record_twcc_remote_stream_observation(
        XbxEngineTwccRemoteStreamObservation {
            observation_id: TWCC_REMOTE_STREAM_OBSERVATION_ID.fetch_add(1, Ordering::Relaxed) + 1,
            ssrc,
            mime_type,
            twcc_ext_id,
            header_extensions,
            rtcp_feedback,
            observed_at_ms: now_ms_f64(),
        },
    );
}

pub(super) fn record_twcc_inbound_extension_observation(
    runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    state: &str,
    ssrc: u32,
    sequence_number: u16,
    expected_ext_id: u8,
    packet_seen_count: u64,
    missing_count: u64,
) {
    RuntimeStatsSink::new(runtime_stats.clone()).record_twcc_inbound_extension_observation(
        XbxEngineTwccExtensionObservation {
            observation_id: TWCC_EXTENSION_OBSERVATION_ID.fetch_add(1, Ordering::Relaxed) + 1,
            state: state.to_string(),
            ssrc,
            sequence_number,
            expected_ext_id,
            packet_seen_count,
            missing_count,
            observed_at_ms: now_ms_f64(),
        },
    );
}

pub(super) struct ControlledTwccFeedbackController {
    feedback_interval: Duration,
    interceptor: LocalTwccInterceptor,
    twcc_observation_id: u64,
    outbound_twcc_feedback_count: u64,
    track_receivers: HashMap<String, RTCRtpReceiverId>,
    remote_twcc_streams: HashMap<u32, ControlledTwccStreamBinding>,
    preferred_video_receiver_id: Option<RTCRtpReceiverId>,
    preferred_video_media_ssrc: Option<u32>,
    pending_feedback_packets: Vec<PendingTwccFeedbackPacket>,
    dropped_pending_feedback_count: u64,
    twcc_packet_bytes_ledger: VecDeque<TwccPacketBytesSample>,
    video_feedback_bootstrap_binding: Option<VideoFeedbackBootstrapBinding>,
}

impl ControlledTwccFeedbackController {
    pub(super) fn new(feedback_interval_ms: u64) -> Self {
        let feedback_interval = Duration::from_millis(feedback_interval_ms.max(1));
        Self {
            feedback_interval,
            interceptor: build_local_twcc_interceptor(feedback_interval),
            twcc_observation_id: 0,
            outbound_twcc_feedback_count: 0,
            track_receivers: HashMap::new(),
            remote_twcc_streams: HashMap::new(),
            preferred_video_receiver_id: None,
            preferred_video_media_ssrc: None,
            pending_feedback_packets: Vec::new(),
            dropped_pending_feedback_count: 0,
            twcc_packet_bytes_ledger: VecDeque::new(),
            video_feedback_bootstrap_binding: None,
        }
    }

    pub(super) fn set_feedback_interval(&mut self, feedback_interval_ms: u64) {
        let interval = Duration::from_millis(feedback_interval_ms.max(1));
        if self.feedback_interval == interval {
            return;
        }
        self.feedback_interval = interval;
        let mut interceptor = build_local_twcc_interceptor(self.feedback_interval);
        for (ssrc, binding) in &self.remote_twcc_streams {
            interceptor.bind_remote_stream(&build_stream_info(
                *ssrc,
                binding.payload_type,
                &binding.mime_type,
                binding.twcc_ext_id,
                &binding.rtcp_feedback,
            ));
        }
        self.interceptor = interceptor;
    }

    pub(super) fn reset(&mut self) {
        self.interceptor = build_local_twcc_interceptor(self.feedback_interval);
        self.twcc_observation_id = 0;
        self.outbound_twcc_feedback_count = 0;
        self.track_receivers.clear();
        self.remote_twcc_streams.clear();
        self.preferred_video_receiver_id = None;
        self.preferred_video_media_ssrc = None;
        self.pending_feedback_packets.clear();
        self.dropped_pending_feedback_count = 0;
        self.twcc_packet_bytes_ledger.clear();
    }

    pub(super) fn feedback_interval_ms(&self) -> u64 {
        self.feedback_interval.as_millis().max(1) as u64
    }

    pub(super) fn register_track_open(
        &mut self,
        track_id: &MediaStreamTrackId,
        receiver_id: RTCRtpReceiverId,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) {
        let track_id = track_id.to_string();
        self.restore_video_feedback_binding_from_bootstrap(&track_id);
        self.track_receivers.insert(track_id.clone(), receiver_id);
        let mut restored_video_target = false;
        for (ssrc, binding) in self.remote_twcc_streams.iter_mut() {
            if binding.track_id != track_id {
                continue;
            }
            binding.receiver_id = Some(receiver_id);
            if !is_audio_mime_type(binding.mime_type.as_str()) {
                self.preferred_video_receiver_id = Some(receiver_id);
                self.preferred_video_media_ssrc = Some(*ssrc);
                restored_video_target = true;
            }
        }
        if restored_video_target {
            RuntimeStatsSink::new(runtime_stats.clone()).record_feedback_target_availability(
                now_ms_f64(),
                VIDEO_TWCC_FEEDBACK_TARGET,
                "ready",
                "feedbackTargetBound",
            );
        }
    }

    pub(super) fn apply_remote_answer_bootstrap(
        &mut self,
        answer_sdp: &str,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) {
        let Some(binding) = parse_video_feedback_bootstrap_binding_from_answer_sdp(answer_sdp)
        else {
            self.video_feedback_bootstrap_binding = None;
            return;
        };
        self.video_feedback_bootstrap_binding = Some(binding.clone());
        if self.remote_twcc_streams.contains_key(&binding.ssrc) {
            return;
        }
        let receiver_id = self.track_receivers.get(&binding.track_id).copied();
        self.interceptor.bind_remote_stream(&build_stream_info(
            binding.ssrc,
            binding.payload_type,
            &binding.mime_type,
            binding.twcc_ext_id,
            &binding.rtcp_feedback,
        ));
        self.remote_twcc_streams.insert(
            binding.ssrc,
            ControlledTwccStreamBinding {
                receiver_id,
                track_id: binding.track_id.clone(),
                mime_type: binding.mime_type.clone(),
                payload_type: binding.payload_type,
                rtcp_feedback: binding.rtcp_feedback.clone(),
                twcc_ext_id: binding.twcc_ext_id,
                packet_seen_count: 0,
                missing_extension_count: 0,
            },
        );
        if receiver_id.is_some() {
            self.preferred_video_media_ssrc = Some(binding.ssrc);
            self.preferred_video_receiver_id = receiver_id;
            RuntimeStatsSink::new(runtime_stats.clone()).record_feedback_target_availability(
                now_ms_f64(),
                "videoRtcpFeedback",
                "ready",
                "feedbackTargetBound",
            );
        }
    }

    pub(super) fn prime_video_feedback_receiver_hint(
        &mut self,
        receiver_id: RTCRtpReceiverId,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) {
        if self.preferred_video_receiver_id.is_some() {
            return;
        }
        let Some(binding) = self.video_feedback_bootstrap_binding.clone() else {
            return;
        };
        self.restore_video_feedback_binding_from_bootstrap(&binding.track_id);
        if let Some(existing) = self.remote_twcc_streams.get_mut(&binding.ssrc) {
            existing.receiver_id = Some(receiver_id);
            self.preferred_video_receiver_id = Some(receiver_id);
            self.preferred_video_media_ssrc = Some(binding.ssrc);
            RuntimeStatsSink::new(runtime_stats.clone()).record_feedback_target_availability(
                now_ms_f64(),
                "videoRtcpFeedback",
                "ready",
                "feedbackTargetBound",
            );
        }
    }

    pub(super) fn unregister_track(
        &mut self,
        track_id: &MediaStreamTrackId,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) {
        let track_id = track_id.to_string();
        self.track_receivers.remove(&track_id);
        self.remote_twcc_streams
            .retain(|_, binding| binding.track_id != track_id);
        self.refresh_preferred_video_target();
        self.sync_feedback_target_availability(runtime_stats, "feedbackTargetUnbound");
    }

    pub(super) fn observe_inbound_rtp(
        &mut self,
        track_id: &MediaStreamTrackId,
        packet: &rtc_rtp::packet::Packet,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        remote_answer_sdp: Option<&str>,
        fallback_mime_type: Option<String>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let track_key = track_id.to_string();
        let mut video_rtcp_feedback_target_became_ready = false;
        if !self.remote_twcc_streams.contains_key(&packet.header.ssrc) {
            let binding_info = resolve_twcc_binding_info(
                remote_answer_sdp,
                packet.header.payload_type,
                fallback_mime_type,
            );
            let negotiated_transport_cc = binding_info.twcc_ext_id.is_some()
                || binding_info
                    .rtcp_feedback
                    .iter()
                    .any(|feedback| feedback.starts_with("transport-cc"));
            if negotiated_transport_cc {
                let mime_type = binding_info
                    .mime_type
                    .clone()
                    .unwrap_or_else(|| format!("track:{track_key}"));
                record_twcc_remote_stream_observation(
                    runtime_stats,
                    packet.header.ssrc,
                    mime_type.clone(),
                    binding_info.twcc_ext_id,
                    binding_info.header_extensions.clone(),
                    binding_info.rtcp_feedback.clone(),
                );
                if let Some(ext_id) = binding_info.twcc_ext_id {
                    let receiver_id = self.track_receivers.get(&track_key).copied();
                    self.interceptor.bind_remote_stream(&build_stream_info(
                        packet.header.ssrc,
                        packet.header.payload_type,
                        &mime_type,
                        ext_id,
                        &binding_info.rtcp_feedback,
                    ));
                    self.remote_twcc_streams.insert(
                        packet.header.ssrc,
                        ControlledTwccStreamBinding {
                            receiver_id,
                            track_id: track_key.clone(),
                            mime_type: mime_type.clone(),
                            payload_type: packet.header.payload_type,
                            rtcp_feedback: binding_info.rtcp_feedback.clone(),
                            twcc_ext_id: ext_id,
                            packet_seen_count: 0,
                            missing_extension_count: 0,
                        },
                    );
                    if !is_audio_mime_type(mime_type.as_str()) {
                        self.video_feedback_bootstrap_binding =
                            Some(VideoFeedbackBootstrapBinding {
                                ssrc: packet.header.ssrc,
                                track_id: track_key.clone(),
                                mime_type: mime_type.clone(),
                                payload_type: packet.header.payload_type,
                                rtcp_feedback: binding_info.rtcp_feedback.clone(),
                                twcc_ext_id: ext_id,
                            });
                    }
                    if !is_audio_mime_type(mime_type.as_str()) {
                        self.preferred_video_media_ssrc = Some(packet.header.ssrc);
                        self.preferred_video_receiver_id = receiver_id;
                        video_rtcp_feedback_target_became_ready |= receiver_id.is_some();
                    }
                }
            }
        }

        let mut seen_observation: Option<(u8, u64, u64)> = None;
        let mut missing_observation: Option<(u8, u64, u64)> = None;
        let mut raw_extension_payload: Option<bytes::Bytes> = None;
        {
            let Some(binding) = self.remote_twcc_streams.get_mut(&packet.header.ssrc) else {
                return Ok(());
            };
            if binding.receiver_id.is_none() {
                binding.receiver_id = self.track_receivers.get(&track_key).copied();
                if !is_audio_mime_type(binding.mime_type.as_str()) && binding.receiver_id.is_some()
                {
                    self.preferred_video_media_ssrc = Some(packet.header.ssrc);
                    self.preferred_video_receiver_id = binding.receiver_id;
                    video_rtcp_feedback_target_became_ready = true;
                }
            }

            binding.packet_seen_count = binding.packet_seen_count.saturating_add(1);
            if let Some(raw_extension) = packet.header.get_extension(binding.twcc_ext_id) {
                raw_extension_payload = Some(raw_extension.clone());
                if binding.packet_seen_count <= 3 {
                    seen_observation = Some((
                        binding.twcc_ext_id,
                        binding.packet_seen_count,
                        binding.missing_extension_count,
                    ));
                }
            } else {
                binding.missing_extension_count = binding.missing_extension_count.saturating_add(1);
                if binding.missing_extension_count <= 3
                    || binding
                        .missing_extension_count
                        .is_multiple_of(TWCC_PACKET_MISS_LOG_INTERVAL)
                {
                    missing_observation = Some((
                        binding.twcc_ext_id,
                        binding.packet_seen_count,
                        binding.missing_extension_count,
                    ));
                }
            }
        }

        if let Some(raw_extension) = raw_extension_payload {
            let mut extension_payload = raw_extension;
            if let Ok(twcc_extension) = TransportCcExtension::unmarshal(&mut extension_payload) {
                self.record_twcc_packet_bytes_sample(
                    twcc_extension.transport_sequence,
                    packet.marshal_size() as u32,
                );
            }
            if let Some((twcc_ext_id, packet_seen_count, missing_extension_count)) =
                seen_observation
            {
                record_twcc_inbound_extension_observation(
                    runtime_stats,
                    "seen",
                    packet.header.ssrc,
                    packet.header.sequence_number,
                    twcc_ext_id,
                    packet_seen_count,
                    missing_extension_count,
                );
            }
            self.interceptor
                .handle_read(TaggedPacket {
                    now: Instant::now(),
                    transport: TransportContext::default(),
                    message: Packet::Rtp(packet.clone()),
                })
                .map_err(|err| {
                    XbxEngineRuntimeError::new(format!(
                        "xbxEngineTwccControlledHandleReadFailed: {err}"
                    ))
                })?;
        } else if let Some((twcc_ext_id, packet_seen_count, missing_extension_count)) =
            missing_observation
        {
            record_twcc_inbound_extension_observation(
                runtime_stats,
                "missing",
                packet.header.ssrc,
                packet.header.sequence_number,
                twcc_ext_id,
                packet_seen_count,
                missing_extension_count,
            );
        }

        if video_rtcp_feedback_target_became_ready {
            RuntimeStatsSink::new(runtime_stats.clone()).record_feedback_target_availability(
                now_ms_f64(),
                "videoRtcpFeedback",
                "ready",
                "feedbackTargetBound",
            );
        }

        Ok(())
    }

    pub(super) fn flush_due_feedback(
        &mut self,
        peer_connection: &mut ControlledPeerConnection,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let now = Instant::now();
        while let Some(timeout) = self.interceptor.poll_timeout() {
            if timeout > now {
                break;
            }
            self.interceptor.handle_timeout(now).map_err(|err| {
                XbxEngineRuntimeError::new(format!(
                    "xbxEngineTwccControlledHandleTimeoutFailed: {err}"
                ))
            })?;
        }

        let mut feedback_packets_by_receiver =
            HashMap::<RTCRtpReceiverId, Vec<Box<dyn rtc::rtcp::Packet>>>::new();
        self.drain_pending_feedback_packets(runtime_stats, &mut feedback_packets_by_receiver);
        while let Some(tagged_packet) = self.interceptor.poll_write() {
            let Packet::Rtcp(rtcp_packets) = tagged_packet.message else {
                continue;
            };
            for packet in rtcp_packets {
                let media_ssrc = packet
                    .as_any()
                    .downcast_ref::<TransportLayerCc>()
                    .map(|twcc| twcc.media_ssrc);
                if let Some(twcc) = packet.as_any().downcast_ref::<TransportLayerCc>() {
                    self.observe_local_feedback(runtime_stats, twcc, media_ssrc);
                }
                self.route_or_queue_feedback_packet(
                    media_ssrc,
                    packet,
                    runtime_stats,
                    &mut feedback_packets_by_receiver,
                );
            }
        }

        for (receiver_id, packets) in feedback_packets_by_receiver {
            let Some(mut receiver) = peer_connection.rtp_receiver(receiver_id) else {
                RuntimeStatsSink::new(runtime_stats.clone()).record_twcc_feedback_send_failure(
                    now_ms_f64(),
                    "xbxEngineTwccControlledReceiverLookupMiss",
                );
                continue;
            };
            receiver.write_rtcp(packets).map_err(|err| {
                let error = format!("xbxEngineTwccControlledWriteRtcpFailed: {err}");
                RuntimeStatsSink::new(runtime_stats.clone())
                    .record_twcc_feedback_send_failure(now_ms_f64(), &error);
                XbxEngineRuntimeError::new(error)
            })?;
            RuntimeStatsSink::new(runtime_stats.clone()).record_feedback_target_availability(
                now_ms_f64(),
                VIDEO_TWCC_FEEDBACK_TARGET,
                "ready",
                "twccSent",
            );
        }

        Ok(())
    }

    pub(super) fn preferred_video_feedback_target(
        &mut self,
    ) -> Option<(RTCRtpReceiverId, Option<u32>)> {
        if self.preferred_video_receiver_id.is_none() {
            self.refresh_preferred_video_target();
        }
        self.preferred_video_receiver_id
            .map(|receiver_id| (receiver_id, self.preferred_video_media_ssrc))
    }

    fn observe_local_feedback(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        twcc: &TransportLayerCc,
        media_ssrc: Option<u32>,
    ) {
        if let Some(ssrc) = media_ssrc {
            if let Some(binding) = self.remote_twcc_streams.get(&ssrc) {
                if is_audio_mime_type(binding.mime_type.as_str()) {
                    crate::xbx_log_warn!(
                        "[xbxengine][twcc] local feedback ignored for non-video stream ssrc={} mime={}",
                        ssrc,
                        binding.mime_type
                    );
                    return;
                }
            }
        }
        self.twcc_observation_id = self.twcc_observation_id.saturating_add(1);
        self.outbound_twcc_feedback_count = self.outbound_twcc_feedback_count.saturating_add(1);
        // 这里记录的是“本地受控 TWCC feedback 已经生成”，用于证明链路确实由我们驱动。
        if let Some(observation) = build_twcc_observation_with_packet_bytes(
            self.twcc_observation_id,
            twcc,
            runtime_stats,
            TWCC_OBSERVATION_SOURCE_LOCAL_FEEDBACK,
            Some(&self.packet_bytes_by_transport_sequence()),
        ) {
            RuntimeStatsSink::new(runtime_stats.clone())
                .record_latest_video_twcc_observation(observation);
        }
    }

    fn record_twcc_packet_bytes_sample(&mut self, transport_sequence: u16, packet_bytes: u32) {
        let observed_at_ms = now_ms_f64();
        self.twcc_packet_bytes_ledger
            .push_back(TwccPacketBytesSample {
                observed_at_ms,
                transport_sequence,
                packet_bytes,
            });
        self.compact_twcc_packet_bytes_ledger(observed_at_ms);
    }

    fn compact_twcc_packet_bytes_ledger(&mut self, now_ms: f64) {
        while self.twcc_packet_bytes_ledger.front().is_some_and(|sample| {
            now_ms - sample.observed_at_ms > TWCC_PACKET_BYTES_LEDGER_WINDOW_MS
        }) {
            self.twcc_packet_bytes_ledger.pop_front();
        }
    }

    fn packet_bytes_by_transport_sequence(&mut self) -> HashMap<u16, u32> {
        let now_ms = now_ms_f64();
        self.compact_twcc_packet_bytes_ledger(now_ms);
        let mut bytes_by_sequence = HashMap::with_capacity(self.twcc_packet_bytes_ledger.len());
        for sample in self.twcc_packet_bytes_ledger.iter() {
            bytes_by_sequence.insert(sample.transport_sequence, sample.packet_bytes);
        }
        bytes_by_sequence
    }

    fn resolve_receiver_id_for_media_ssrc(&mut self, media_ssrc: u32) -> Option<RTCRtpReceiverId> {
        let binding = self.remote_twcc_streams.get_mut(&media_ssrc)?;
        if binding.receiver_id.is_none() {
            binding.receiver_id = self.track_receivers.get(&binding.track_id).copied();
        }
        binding.receiver_id
    }

    fn route_or_queue_feedback_packet(
        &mut self,
        media_ssrc: Option<u32>,
        packet: Box<dyn rtc::rtcp::Packet>,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        feedback_packets_by_receiver: &mut HashMap<
            RTCRtpReceiverId,
            Vec<Box<dyn rtc::rtcp::Packet>>,
        >,
    ) {
        if let Some(receiver_id) =
            media_ssrc.and_then(|ssrc| self.resolve_receiver_id_for_media_ssrc(ssrc))
        {
            feedback_packets_by_receiver
                .entry(receiver_id)
                .or_default()
                .push(packet);
            return;
        }

        // 没有 receiver 映射时先缓存，避免把已生成的 feedback 静默丢弃。
        if self.pending_feedback_packets.len() >= TWCC_PENDING_FEEDBACK_MAX {
            let dropped = self.pending_feedback_packets.remove(0);
            self.dropped_pending_feedback_count =
                self.dropped_pending_feedback_count.saturating_add(1);
            crate::xbx_log_warn!(
                "[xbxengine][twcc] pending feedback queue full; dropping oldest packet media_ssrc={:?} dropped_total={}",
                dropped.media_ssrc,
                self.dropped_pending_feedback_count
            );
        }
        crate::xbx_log_warn!(
            "[xbxengine][twcc] queue feedback packet without receiver mapping media_ssrc={:?} pending={}",
            media_ssrc,
            self.pending_feedback_packets.len().saturating_add(1)
        );
        RuntimeStatsSink::new(runtime_stats.clone()).record_twcc_receiver_mapping_missing(
            now_ms_f64(),
            media_ssrc,
            self.pending_feedback_packets.len().saturating_add(1),
            self.dropped_pending_feedback_count,
        );
        self.pending_feedback_packets
            .push(PendingTwccFeedbackPacket { media_ssrc, packet });
    }

    fn drain_pending_feedback_packets(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        feedback_packets_by_receiver: &mut HashMap<
            RTCRtpReceiverId,
            Vec<Box<dyn rtc::rtcp::Packet>>,
        >,
    ) {
        if self.pending_feedback_packets.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.pending_feedback_packets);
        for pending_packet in pending {
            self.route_or_queue_feedback_packet(
                pending_packet.media_ssrc,
                pending_packet.packet,
                runtime_stats,
                feedback_packets_by_receiver,
            );
        }
    }

    fn refresh_preferred_video_target(&mut self) {
        self.preferred_video_receiver_id = None;
        self.preferred_video_media_ssrc = None;
        for (ssrc, binding) in self.remote_twcc_streams.iter_mut() {
            if is_audio_mime_type(binding.mime_type.as_str()) {
                continue;
            }
            if binding.receiver_id.is_none() {
                binding.receiver_id = self.track_receivers.get(&binding.track_id).copied();
            }
            if let Some(receiver_id) = binding.receiver_id {
                self.preferred_video_receiver_id = Some(receiver_id);
                self.preferred_video_media_ssrc = Some(*ssrc);
                break;
            }
        }
    }

    fn sync_feedback_target_availability(
        &self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        unbound_reason: &str,
    ) {
        let sink = RuntimeStatsSink::new(runtime_stats.clone());
        if self.preferred_video_receiver_id.is_some() {
            sink.record_feedback_target_availability(
                now_ms_f64(),
                VIDEO_TWCC_FEEDBACK_TARGET,
                "ready",
                "feedbackTargetBound",
            );
        } else {
            sink.record_feedback_target_availability(
                now_ms_f64(),
                VIDEO_TWCC_FEEDBACK_TARGET,
                "unbound",
                unbound_reason,
            );
        }
    }

    fn restore_video_feedback_binding_from_bootstrap(&mut self, track_id: &str) {
        let Some(binding) = self.video_feedback_bootstrap_binding.clone() else {
            return;
        };
        if binding.track_id != track_id || self.remote_twcc_streams.contains_key(&binding.ssrc) {
            return;
        }
        self.interceptor.bind_remote_stream(&build_stream_info(
            binding.ssrc,
            binding.payload_type,
            &binding.mime_type,
            binding.twcc_ext_id,
            &binding.rtcp_feedback,
        ));
        self.remote_twcc_streams.insert(
            binding.ssrc,
            ControlledTwccStreamBinding {
                receiver_id: self.track_receivers.get(track_id).copied(),
                track_id: binding.track_id,
                mime_type: binding.mime_type,
                payload_type: binding.payload_type,
                rtcp_feedback: binding.rtcp_feedback,
                twcc_ext_id: binding.twcc_ext_id,
                packet_seen_count: 0,
                missing_extension_count: 0,
            },
        );
    }
}

impl Default for ControlledTwccFeedbackController {
    fn default() -> Self {
        Self::new(100)
    }
}

fn build_local_twcc_interceptor(feedback_interval: Duration) -> LocalTwccInterceptor {
    Registry::new()
        .with(
            TwccReceiverBuilder::new()
                .with_interval(feedback_interval)
                .build(),
        )
        .build()
}

fn build_stream_info(
    ssrc: u32,
    payload_type: u8,
    mime_type: &str,
    twcc_ext_id: u8,
    rtcp_feedback: &[String],
) -> StreamInfo {
    StreamInfo {
        ssrc,
        payload_type,
        mime_type: mime_type.to_string(),
        rtp_header_extensions: vec![RTPHeaderExtension {
            uri: TRANSPORT_CC_URI.to_string(),
            id: twcc_ext_id as u16,
        }],
        rtcp_feedback: rtcp_feedback
            .iter()
            .map(|value| parse_rtcp_feedback(value))
            .collect(),
        ..Default::default()
    }
}

fn parse_rtcp_feedback(value: &str) -> RTCPFeedback {
    let mut parts = value.splitn(2, ':');
    RTCPFeedback {
        typ: parts.next().unwrap_or_default().to_string(),
        parameter: parts.next().unwrap_or_default().to_string(),
    }
}

fn is_audio_mime_type(mime_type: &str) -> bool {
    let normalized = mime_type.to_ascii_lowercase();
    normalized.starts_with("audio/")
        || normalized.starts_with("opus/")
        || normalized.starts_with("pcmu/")
        || normalized.starts_with("pcma/")
        || normalized.starts_with("g722/")
}

fn resolve_twcc_binding_info(
    remote_answer_sdp: Option<&str>,
    payload_type: u8,
    fallback_mime_type: Option<String>,
) -> TwccSdpBindingInfo {
    let Some(answer_sdp) = remote_answer_sdp else {
        return TwccSdpBindingInfo {
            mime_type: fallback_mime_type,
            ..Default::default()
        };
    };
    let mut binding = parse_twcc_binding_info_from_answer_sdp(answer_sdp, payload_type);
    if binding.mime_type.is_none() {
        binding.mime_type = fallback_mime_type;
    }
    binding
}

fn parse_video_feedback_bootstrap_binding_from_answer_sdp(
    sdp: &str,
) -> Option<VideoFeedbackBootstrapBinding> {
    let mut in_video_section = false;
    let mut current_payload_types = Vec::<u8>::new();
    let mut selected_track_id: Option<String> = None;
    let mut selected_ssrc: Option<u32> = None;
    let mut selected_payload_type: Option<u8> = None;
    for line in sdp.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("m=") {
            in_video_section = rest.starts_with("video ");
            current_payload_types.clear();
            if in_video_section {
                current_payload_types = rest
                    .split_whitespace()
                    .skip(3)
                    .filter_map(|token| token.parse::<u8>().ok())
                    .collect();
                selected_payload_type = current_payload_types.first().copied();
            }
            continue;
        }
        if !in_video_section {
            continue;
        }
        if selected_track_id.is_none() {
            if let Some(msid) = line.strip_prefix("a=msid:") {
                selected_track_id = msid.split_whitespace().nth(1).map(str::to_string);
            }
            continue;
        }
        if selected_ssrc.is_none() {
            if let Some(group) = line.strip_prefix("a=ssrc-group:FID ") {
                selected_ssrc = group
                    .split_whitespace()
                    .next()
                    .and_then(|value| value.parse::<u32>().ok());
            }
            continue;
        }
        if selected_ssrc.is_none() {
            if let Some(ssrc_line) = line.strip_prefix("a=ssrc:") {
                selected_ssrc = ssrc_line
                    .split_whitespace()
                    .next()
                    .and_then(|value| value.parse::<u32>().ok());
            }
        }
    }
    let selected_payload_type = selected_payload_type?;
    let info = parse_twcc_binding_info_from_answer_sdp(sdp, selected_payload_type);
    let twcc_ext_id = info.twcc_ext_id?;
    let mime_type = info.mime_type?;
    if is_audio_mime_type(mime_type.as_str()) {
        return None;
    }
    Some(VideoFeedbackBootstrapBinding {
        ssrc: selected_ssrc?,
        track_id: selected_track_id?,
        mime_type,
        payload_type: selected_payload_type,
        rtcp_feedback: info.rtcp_feedback,
        twcc_ext_id,
    })
}

pub(super) fn parse_twcc_binding_info_from_answer_sdp(
    sdp: &str,
    payload_type: u8,
) -> TwccSdpBindingInfo {
    let payload_type_text = payload_type.to_string();
    let mut session_level_info = TwccSdpBindingInfo::default();
    let mut current_media_info = TwccSdpBindingInfo::default();
    let mut current_media_payload_types = Vec::<String>::new();
    let mut matched_media_info = None;
    let mut inside_media_section = false;

    let finalize_media_section =
        |matched_media_info: &mut Option<TwccSdpBindingInfo>,
         current_media_info: &mut TwccSdpBindingInfo,
         current_media_payload_types: &mut Vec<String>| {
            if current_media_payload_types
                .iter()
                .any(|value| value == &payload_type_text)
            {
                *matched_media_info = Some(current_media_info.clone());
            }
            current_media_info.header_extensions.clear();
            current_media_info.rtcp_feedback.clear();
            current_media_info.mime_type = None;
            current_media_info.twcc_ext_id = None;
            current_media_payload_types.clear();
        };

    for line in sdp.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("m=") {
            if inside_media_section {
                finalize_media_section(
                    &mut matched_media_info,
                    &mut current_media_info,
                    &mut current_media_payload_types,
                );
            }
            inside_media_section = true;
            let mut parts = rest.split_whitespace();
            let _media_kind = parts.next();
            let _port = parts.next();
            let _proto = parts.next();
            current_media_payload_types.extend(parts.map(|value| value.to_string()));
            continue;
        }

        let target_info = if inside_media_section {
            &mut current_media_info
        } else {
            &mut session_level_info
        };

        if let Some(rest) = line.strip_prefix("a=extmap:") {
            let mut parts = rest.split_whitespace();
            if let (Some(id_text), Some(uri)) = (parts.next(), parts.next()) {
                let normalized_id = id_text.split('/').next().unwrap_or(id_text);
                target_info
                    .header_extensions
                    .push(format!("{uri}#{normalized_id}"));
                if uri == TRANSPORT_CC_URI {
                    target_info.twcc_ext_id = normalized_id.parse::<u8>().ok();
                }
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("a=rtcp-fb:") {
            let mut parts = rest.split_whitespace();
            if let (Some(pt), Some(kind)) = (parts.next(), parts.next()) {
                if pt == "*" || pt == payload_type_text {
                    let parameter = parts.collect::<Vec<_>>().join(" ");
                    if parameter.is_empty() {
                        target_info.rtcp_feedback.push(format!("{kind}:"));
                    } else {
                        target_info
                            .rtcp_feedback
                            .push(format!("{kind}:{parameter}"));
                    }
                }
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("a=rtpmap:") {
            let mut parts = rest.split_whitespace();
            if let (Some(pt), Some(codec)) = (parts.next(), parts.next()) {
                if pt == payload_type_text {
                    target_info.mime_type =
                        Some(codec.split('/').take(2).collect::<Vec<_>>().join("/"));
                }
            }
        }
    }

    if inside_media_section {
        finalize_media_section(
            &mut matched_media_info,
            &mut current_media_info,
            &mut current_media_payload_types,
        );
    }

    let mut info = session_level_info;
    if let Some(media_info) = matched_media_info {
        if media_info.twcc_ext_id.is_some() {
            info.twcc_ext_id = media_info.twcc_ext_id;
        }
        if !media_info.header_extensions.is_empty() {
            info.header_extensions = media_info.header_extensions;
        }
        if !media_info.rtcp_feedback.is_empty() {
            info.rtcp_feedback = media_info.rtcp_feedback;
        }
        if media_info.mime_type.is_some() {
            info.mime_type = media_info.mime_type;
        }
    }
    info
}

#[cfg(test)]
#[path = "twcc_feedback.test.rs"]
mod tests;
