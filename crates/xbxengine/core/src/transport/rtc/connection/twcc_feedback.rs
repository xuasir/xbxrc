use std::collections::HashMap;
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
use rtc::shared::TransportContext;

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::connection::builder::ControlledPeerConnection;
use crate::transport::rtc::connection::transport_metrics::{
    build_twcc_observation, TWCC_OBSERVATION_SOURCE_LOCAL_FEEDBACK,
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
    twcc_ext_id: u8,
    packet_seen_count: u64,
    missing_extension_count: u64,
}

struct PendingTwccFeedbackPacket {
    media_ssrc: Option<u32>,
    packet: Box<dyn rtc::rtcp::Packet>,
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
        }
    }

    pub(super) fn set_feedback_interval(&mut self, feedback_interval_ms: u64) {
        let interval = Duration::from_millis(feedback_interval_ms.max(1));
        if self.feedback_interval == interval {
            return;
        }
        self.feedback_interval = interval;
        self.reset();
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
    }

    pub(super) fn register_track_open(
        &mut self,
        track_id: &MediaStreamTrackId,
        receiver_id: RTCRtpReceiverId,
    ) {
        let track_id = track_id.to_string();
        self.track_receivers.insert(track_id.clone(), receiver_id);
        for (ssrc, binding) in self.remote_twcc_streams.iter_mut() {
            if binding.track_id != track_id {
                continue;
            }
            binding.receiver_id = Some(receiver_id);
            if !is_audio_mime_type(binding.mime_type.as_str()) {
                self.preferred_video_receiver_id = Some(receiver_id);
                self.preferred_video_media_ssrc = Some(*ssrc);
            }
        }
    }

    pub(super) fn unregister_track(&mut self, track_id: &MediaStreamTrackId) {
        let track_id = track_id.to_string();
        self.track_receivers.remove(&track_id);
        self.remote_twcc_streams
            .retain(|_, binding| binding.track_id != track_id);
        if self
            .remote_twcc_streams
            .values()
            .all(|binding| binding.track_id != track_id)
        {
            self.refresh_preferred_video_target();
        }
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
                            twcc_ext_id: ext_id,
                            packet_seen_count: 0,
                            missing_extension_count: 0,
                        },
                    );
                    if !is_audio_mime_type(mime_type.as_str()) {
                        self.preferred_video_media_ssrc = Some(packet.header.ssrc);
                        self.preferred_video_receiver_id = receiver_id;
                    }
                }
            }
        }

        let Some(binding) = self.remote_twcc_streams.get_mut(&packet.header.ssrc) else {
            return Ok(());
        };
        if binding.receiver_id.is_none() {
            binding.receiver_id = self.track_receivers.get(&track_key).copied();
            if !is_audio_mime_type(binding.mime_type.as_str()) && binding.receiver_id.is_some() {
                self.preferred_video_media_ssrc = Some(packet.header.ssrc);
                self.preferred_video_receiver_id = binding.receiver_id;
            }
        }

        binding.packet_seen_count = binding.packet_seen_count.saturating_add(1);
        if packet.header.get_extension(binding.twcc_ext_id).is_some() {
            if binding.packet_seen_count <= 3 {
                record_twcc_inbound_extension_observation(
                    runtime_stats,
                    "seen",
                    packet.header.ssrc,
                    packet.header.sequence_number,
                    binding.twcc_ext_id,
                    binding.packet_seen_count,
                    binding.missing_extension_count,
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
        } else {
            binding.missing_extension_count = binding.missing_extension_count.saturating_add(1);
            if binding.missing_extension_count <= 3
                || binding
                    .missing_extension_count
                    .is_multiple_of(TWCC_PACKET_MISS_LOG_INTERVAL)
            {
                record_twcc_inbound_extension_observation(
                    runtime_stats,
                    "missing",
                    packet.header.ssrc,
                    packet.header.sequence_number,
                    binding.twcc_ext_id,
                    binding.packet_seen_count,
                    binding.missing_extension_count,
                );
            }
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
        self.drain_pending_feedback_packets(&mut feedback_packets_by_receiver);
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
                    &mut feedback_packets_by_receiver,
                );
            }
        }

        for (receiver_id, packets) in feedback_packets_by_receiver {
            let Some(mut receiver) = peer_connection.rtp_receiver(receiver_id) else {
                continue;
            };
            receiver.write_rtcp(packets).map_err(|err| {
                XbxEngineRuntimeError::new(format!("xbxEngineTwccControlledWriteRtcpFailed: {err}"))
            })?;
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
        if let Some(observation) = build_twcc_observation(
            self.twcc_observation_id,
            twcc,
            runtime_stats,
            TWCC_OBSERVATION_SOURCE_LOCAL_FEEDBACK,
        ) {
            RuntimeStatsSink::new(runtime_stats.clone())
                .record_latest_video_twcc_observation(observation);
        }
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
        self.pending_feedback_packets
            .push(PendingTwccFeedbackPacket { media_ssrc, packet });
    }

    fn drain_pending_feedback_packets(
        &mut self,
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
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    use rtc::interceptor::{Interceptor, Packet};
    use rtc::media_stream::MediaStreamTrackId;
    use rtc::rtp_transceiver::RTCRtpReceiverId;
    use rtc::sansio::Protocol;
    use rtc::shared::marshal::Marshal;
    use rtc::shared::TransportContext;
    use rtc_rtcp::transport_feedbacks::transport_layer_cc::TransportLayerCc;
    use rtc_rtp::extension::transport_cc_extension::TransportCcExtension;

    use super::{
        build_local_twcc_interceptor, parse_twcc_binding_info_from_answer_sdp,
        ControlledTwccFeedbackController,
    };
    use crate::XbxEngineMediaRuntimeStats;

    fn make_rtp_packet_with_twcc(
        ssrc: u32,
        seq: u16,
        twcc_seq: u16,
        hdr_ext_id: u8,
    ) -> rtc_rtp::packet::Packet {
        let mut pkt = rtc_rtp::packet::Packet {
            header: rtc_rtp::header::Header {
                ssrc,
                sequence_number: seq,
                payload_type: 124,
                ..Default::default()
            },
            payload: vec![0u8; 64].into(),
        };
        let ext = TransportCcExtension {
            transport_sequence: twcc_seq,
        };
        let payload = ext.marshal().unwrap();
        pkt.header
            .set_extension(hdr_ext_id, payload.freeze())
            .unwrap();
        pkt
    }

    #[test]
    fn parse_twcc_binding_info_extracts_ext_feedback_and_codec() {
        let sdp = concat!(
            "m=video 9 UDP/TLS/RTP/SAVPF 124\r\n",
            "a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01\r\n",
            "a=rtpmap:124 H264/90000\r\n",
            "a=rtcp-fb:124 transport-cc\r\n",
        );
        let info = parse_twcc_binding_info_from_answer_sdp(sdp, 124);
        assert_eq!(info.twcc_ext_id, Some(3));
        assert_eq!(info.mime_type.as_deref(), Some("H264/90000"));
        assert!(info
            .rtcp_feedback
            .iter()
            .any(|feedback| feedback == "transport-cc:"));
    }

    #[test]
    fn parse_twcc_binding_info_uses_matching_media_section() {
        let sdp = concat!(
            "a=extmap:9 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01\r\n",
            "m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n",
            "a=extmap:1 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01\r\n",
            "a=rtpmap:111 opus/48000/2\r\n",
            "a=rtcp-fb:111 transport-cc\r\n",
            "m=video 9 UDP/TLS/RTP/SAVPF 124\r\n",
            "a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01\r\n",
            "a=rtpmap:124 H264/90000\r\n",
            "a=rtcp-fb:124 transport-cc\r\n",
            "a=rtcp-fb:124 nack pli\r\n",
        );

        let info = parse_twcc_binding_info_from_answer_sdp(sdp, 124);

        assert_eq!(info.twcc_ext_id, Some(3));
        assert_eq!(info.mime_type.as_deref(), Some("H264/90000"));
        assert!(info.header_extensions.iter().any(|extension| extension
            == "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01#3"));
        assert!(info
            .rtcp_feedback
            .iter()
            .any(|feedback| feedback == "transport-cc:"));
        assert!(info
            .rtcp_feedback
            .iter()
            .any(|feedback| feedback == "nack:pli"));
        assert!(!info
            .header_extensions
            .iter()
            .any(|extension| extension.ends_with("#1")));
    }

    #[test]
    fn controlled_twcc_controller_emits_local_feedback_observation() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut controller = ControlledTwccFeedbackController::new(1);
        let track_id: MediaStreamTrackId = "video".to_string();
        let ssrc = 0x22334455;
        let answer_sdp = concat!(
            "m=video 9 UDP/TLS/RTP/SAVPF 124\r\n",
            "a=extmap:5 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01\r\n",
            "a=rtpmap:124 H264/90000\r\n",
            "a=rtcp-fb:124 transport-cc\r\n",
        );

        let receiver_id = None;
        if let Some(receiver_id) = receiver_id {
            controller.register_track_open(&track_id, receiver_id);
        }

        let packet = make_rtp_packet_with_twcc(ssrc, 1, 7, 5);
        controller
            .observe_inbound_rtp(
                &track_id,
                &packet,
                &runtime_stats,
                Some(answer_sdp),
                Some("video/H264".to_string()),
            )
            .unwrap();
        assert!(controller.remote_twcc_streams.contains_key(&ssrc));
        assert!(controller.interceptor.poll_timeout().is_some());
        thread::sleep(Duration::from_millis(10));
        controller
            .interceptor
            .handle_timeout(Instant::now())
            .unwrap();
        while let Some(tagged_packet) = controller.interceptor.poll_write() {
            let Packet::Rtcp(rtcp_packets) = tagged_packet.message else {
                continue;
            };
            for packet in rtcp_packets {
                if let Some(twcc) = packet.as_any().downcast_ref::<TransportLayerCc>() {
                    controller.observe_local_feedback(&runtime_stats, twcc, Some(ssrc));
                }
            }
        }

        let stats = runtime_stats.lock().unwrap();
        assert_eq!(
            stats
                .latest_video_twcc_observation
                .as_ref()
                .map(|observation| observation.source.as_str()),
            Some("local-feedback")
        );
    }

    #[test]
    fn register_track_open_backfills_existing_video_binding() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut controller = ControlledTwccFeedbackController::new(1);
        let track_id: MediaStreamTrackId = "video".to_string();
        let ssrc = 0x33445566;
        let answer_sdp = concat!(
            "m=video 9 UDP/TLS/RTP/SAVPF 124\r\n",
            "a=extmap:5 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01\r\n",
            "a=rtpmap:124 H264/90000\r\n",
            "a=rtcp-fb:124 transport-cc\r\n",
        );
        let packet = make_rtp_packet_with_twcc(ssrc, 1, 7, 5);

        controller
            .observe_inbound_rtp(
                &track_id,
                &packet,
                &runtime_stats,
                Some(answer_sdp),
                Some("video/H264".to_string()),
            )
            .unwrap();

        let binding = controller.remote_twcc_streams.get(&ssrc).unwrap();
        assert!(binding.receiver_id.is_none());
        assert_eq!(controller.preferred_video_receiver_id, None);
        assert_eq!(controller.preferred_video_media_ssrc, Some(ssrc));

        let receiver_id = RTCRtpReceiverId::default();
        controller.register_track_open(&track_id, receiver_id);

        let binding = controller.remote_twcc_streams.get(&ssrc).unwrap();
        assert_eq!(binding.receiver_id, Some(receiver_id));
        assert_eq!(controller.preferred_video_receiver_id, Some(receiver_id));
        assert_eq!(controller.preferred_video_media_ssrc, Some(ssrc));
        assert_eq!(
            controller.preferred_video_feedback_target(),
            Some((receiver_id, Some(ssrc)))
        );
    }

    #[test]
    fn local_twcc_interceptor_builds_feedback_after_timeout() {
        let mut interceptor = build_local_twcc_interceptor(Duration::from_millis(1));
        interceptor.bind_remote_stream(&super::build_stream_info(
            12345,
            124,
            "video/H264",
            5,
            &["transport-cc:".to_string()],
        ));
        interceptor
            .handle_read(rtc::interceptor::TaggedPacket {
                now: Instant::now(),
                transport: TransportContext::default(),
                message: Packet::Rtp(make_rtp_packet_with_twcc(12345, 1, 9, 5)),
            })
            .unwrap();
        thread::sleep(Duration::from_millis(2));
        interceptor.handle_timeout(Instant::now()).unwrap();
        let mut emitted = false;
        while let Some(packet) = interceptor.poll_write() {
            if let Packet::Rtcp(_) = packet.message {
                emitted = true;
            }
        }
        assert!(emitted);
    }

    #[test]
    fn twcc_inbound_extension_counters_are_scoped_per_ssrc() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut controller = ControlledTwccFeedbackController::new(1);
        let track_id: MediaStreamTrackId = "video".to_string();
        let answer_sdp = concat!(
            "m=video 9 UDP/TLS/RTP/SAVPF 124\r\n",
            "a=extmap:5 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01\r\n",
            "a=rtpmap:124 H264/90000\r\n",
            "a=rtcp-fb:124 transport-cc\r\n",
        );
        let ssrc_a = 0x1001u32;
        let ssrc_b = 0x1002u32;

        let mut missing_pkt = make_rtp_packet_with_twcc(ssrc_a, 3, 11, 5);
        missing_pkt.header.extensions.clear();

        controller
            .observe_inbound_rtp(
                &track_id,
                &make_rtp_packet_with_twcc(ssrc_a, 1, 9, 5),
                &runtime_stats,
                Some(answer_sdp),
                Some("video/H264".to_string()),
            )
            .unwrap();
        controller
            .observe_inbound_rtp(
                &track_id,
                &make_rtp_packet_with_twcc(ssrc_a, 2, 10, 5),
                &runtime_stats,
                Some(answer_sdp),
                Some("video/H264".to_string()),
            )
            .unwrap();
        controller
            .observe_inbound_rtp(
                &track_id,
                &missing_pkt,
                &runtime_stats,
                Some(answer_sdp),
                Some("video/H264".to_string()),
            )
            .unwrap();
        controller
            .observe_inbound_rtp(
                &track_id,
                &make_rtp_packet_with_twcc(ssrc_b, 1, 20, 5),
                &runtime_stats,
                Some(answer_sdp),
                Some("video/H264".to_string()),
            )
            .unwrap();

        let binding_a = controller.remote_twcc_streams.get(&ssrc_a).unwrap();
        let binding_b = controller.remote_twcc_streams.get(&ssrc_b).unwrap();
        assert_eq!(binding_a.packet_seen_count, 3);
        assert_eq!(binding_a.missing_extension_count, 1);
        assert_eq!(binding_b.packet_seen_count, 1);
        assert_eq!(binding_b.missing_extension_count, 0);
    }

    #[test]
    fn unroutable_feedback_packets_are_queued_instead_of_silently_dropped() {
        let mut controller = ControlledTwccFeedbackController::new(1);
        let mut routed = HashMap::<RTCRtpReceiverId, Vec<Box<dyn rtc::rtcp::Packet>>>::new();
        let packet: Box<dyn rtc::rtcp::Packet> = Box::new(TransportLayerCc {
            media_ssrc: 0x4455,
            ..Default::default()
        });

        controller.route_or_queue_feedback_packet(Some(0x4455), packet, &mut routed);

        assert!(routed.is_empty());
        assert_eq!(controller.pending_feedback_packets.len(), 1);
        assert_eq!(
            controller.pending_feedback_packets[0].media_ssrc,
            Some(0x4455)
        );
    }
}
