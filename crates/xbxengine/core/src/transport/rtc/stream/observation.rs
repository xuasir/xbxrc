use crate::transport::rtc::stats::now_ms_f64;
use crate::transport::rtc::stream::packet_router::{RtcMediaRouteDecision, RtcMediaRouteLabel};
use crate::transport::rtc::stream::packet_types::{RtcMediaIngressPacket, RtcRtpPacketMeta};
use crate::transport::rtc::stream::runtime_state::RtcMediaRuntimeState;
use crate::XbxEngineMediaRuntimeStats;
use std::sync::{Arc, Mutex};

pub(super) fn apply_ingress_observation(
    state: &RtcMediaRuntimeState,
    packet: &RtcMediaIngressPacket,
    route: &RtcMediaRouteDecision,
    rtp_meta: Option<&RtcRtpPacketMeta>,
    runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
) {
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.latest_observation_label = Some("rtcMediaPacketObserved".to_string());
        stats.latest_observation_summary =
            Some(build_observation_summary(state, packet, route, rtp_meta));
        let now_ms = now_ms_f64();
        if matches!(
            route.label,
            RtcMediaRouteLabel::PrimaryVideo | RtcMediaRouteLabel::RepairVideo
        ) {
            if stats.first_video_packet_arrival_time_ms.is_none() {
                stats.first_video_packet_arrival_time_ms = Some(now_ms);
            }
            stats.latest_video_packet_arrival_time_ms = Some(now_ms);
            stats.latest_video_packet_arrival_rtp_timestamp = rtp_meta.map(|meta| meta.timestamp);
        } else if route.label == RtcMediaRouteLabel::Audio {
            if stats.first_audio_packet_arrival_time_ms.is_none() {
                stats.first_audio_packet_arrival_time_ms = Some(now_ms);
            }
            stats.latest_audio_packet_arrival_time_ms = Some(now_ms);
        }
    }
}

pub(super) fn build_observation_summary(
    state: &RtcMediaRuntimeState,
    packet: &RtcMediaIngressPacket,
    route: &RtcMediaRouteDecision,
    rtp_meta: Option<&RtcRtpPacketMeta>,
) -> String {
    let source = packet
        .stream_identity
        .track_hint()
        .map(|track_id| format!("track:{track_id}"))
        .unwrap_or_else(|| "unknown".to_string());
    let mut summary = format!(
        "phase1 media ingress route={} reason={} bytes={} identity={} source={} rtp_count={} rtp_bytes={} rtcp_count={} rtcp_bytes={} primary_video_count={} repair_video_count={} audio_count={} unknown_count={}",
        route.label.as_str(),
        route.reason,
        packet.byte_len,
        packet.stream_identity.summary(),
        source,
        state.inbound_rtp_count,
        state.inbound_rtp_bytes,
        state.inbound_rtcp_count,
        state.inbound_rtcp_bytes,
        state.inbound_primary_video_count,
        state.inbound_repair_video_count,
        state.inbound_audio_count,
        state.inbound_unknown_count
    );
    if let Some(meta) = rtp_meta {
        summary.push_str(&format!(
            " ssrc={} pt={} seq={} ts={} marker={}",
            meta.ssrc, meta.payload_type, meta.sequence_number, meta.timestamp, meta.marker
        ));
    }
    summary
}
