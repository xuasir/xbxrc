use crate::transport::rtc::stream::packet_router::{RtcMediaRouteLabel, RtcPayloadRouteMap};
use crate::transport::rtc::stream::packet_types::{RtcMediaIngressPacket, RtcRtpPacketMeta};

pub(crate) trait RtcMediaSink: Send + Sync {
    fn apply_payload_route_map(&mut self, _payload_route_map: Option<RtcPayloadRouteMap>) {}

    fn on_raw_packet(
        &mut self,
        _packet: &RtcMediaIngressPacket,
        _route_label: RtcMediaRouteLabel,
        _route_reason: &str,
        _rtp_meta: Option<&RtcRtpPacketMeta>,
    );

    /// 定时驱动点，由外部 tick task 定期调用，用于排空背压队列中的积压包。
    fn on_tick(&mut self, _now: std::time::Instant) {}
}

pub(crate) trait RtcRtcpSendPort: Send + Sync {
    fn send_rtcp(&self, _payload: &[u8]);
}

#[derive(Default)]
pub(crate) struct NullRtcMediaSink;

impl RtcMediaSink for NullRtcMediaSink {
    fn apply_payload_route_map(&mut self, _payload_route_map: Option<RtcPayloadRouteMap>) {}

    fn on_raw_packet(
        &mut self,
        _packet: &RtcMediaIngressPacket,
        _route_label: RtcMediaRouteLabel,
        _route_reason: &str,
        _rtp_meta: Option<&RtcRtpPacketMeta>,
    ) {
    }
}
