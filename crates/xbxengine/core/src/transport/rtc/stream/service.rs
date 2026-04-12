use crate::transport::rtc::stream::observation::apply_ingress_observation;
use crate::transport::rtc::stream::packet_router::{
    classify_packet, parse_payload_route_map_from_answer, RtcPayloadRouteMap,
};
use crate::transport::rtc::stream::packet_types::{
    MediaPacketKind, RtcMediaIngressPacket, RtcMediaPacketSource, RtcRtpPacketMeta,
};
use crate::transport::rtc::stream::runtime_state::{RtcMediaIngressSnapshot, RtcMediaRuntimeState};
use crate::transport::rtc::stream::sink::{NullRtcMediaSink, RtcMediaSink};
use crate::XbxEngineMediaRuntimeStats;
use std::sync::{Arc, Mutex};

pub(crate) struct RtcMediaService {
    state: RtcMediaRuntimeState,
    payload_route_map: Option<RtcPayloadRouteMap>,
    sink: Box<dyn RtcMediaSink>,
}

impl Default for RtcMediaService {
    fn default() -> Self {
        Self {
            state: RtcMediaRuntimeState::default(),
            payload_route_map: None,
            sink: Box::new(NullRtcMediaSink),
        }
    }
}

impl RtcMediaService {
    pub(crate) fn set_sink(&mut self, sink: Box<dyn RtcMediaSink>) {
        self.sink = sink;
    }

    pub(crate) fn reset(&mut self) {
        self.state = RtcMediaRuntimeState::default();
        self.payload_route_map = None;
    }

    // 兼容第一阶段旧调用点；后续连接层可直接切到 observe_ingress_packet。
    #[allow(dead_code)]
    pub(crate) fn observe_raw_packet(
        &mut self,
        packet_kind: MediaPacketKind,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) {
        let packet = RtcMediaIngressPacket::new(packet_kind, 0, RtcMediaPacketSource::Unknown);
        self.observe_ingress_packet(packet, None, runtime_stats);
    }

    pub(crate) fn observe_ingress_packet(
        &mut self,
        packet: RtcMediaIngressPacket,
        rtp_meta: Option<RtcRtpPacketMeta>,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) {
        let packet = packet.with_rtp_meta(rtp_meta.as_ref());
        let route = classify_packet(&packet, rtp_meta.as_ref(), self.payload_route_map.as_ref());
        self.state
            .record_ingress(&packet, &route, rtp_meta.as_ref());
        self.sink
            .on_raw_packet(&packet, route.label, &route.reason, rtp_meta.as_ref());
        apply_ingress_observation(
            &self.state,
            &packet,
            &route,
            rtp_meta.as_ref(),
            runtime_stats,
        );
    }

    pub(crate) fn snapshot(&self) -> RtcMediaIngressSnapshot {
        RtcMediaIngressSnapshot::from(&self.state)
    }

    pub(crate) fn apply_remote_answer_sdp(&mut self, answer_sdp: &str) {
        self.payload_route_map = parse_payload_route_map_from_answer(answer_sdp);
        self.sink
            .apply_payload_route_map(self.payload_route_map.clone());
    }
}

#[cfg(test)]
#[path = "service.test.rs"]
mod tests;
