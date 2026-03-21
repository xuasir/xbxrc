use crate::transport::rtc::media::packet_router::RtcMediaRouteLabel;
use crate::transport::rtc::media::packet_types::{RtcMediaIngressPacket, RtcRtpPacketMeta};

pub(crate) trait RtcMediaSink: Send + Sync {
    fn on_raw_packet(
        &mut self,
        _packet: &RtcMediaIngressPacket,
        _route_label: RtcMediaRouteLabel,
        _route_reason: &str,
        _rtp_meta: Option<&RtcRtpPacketMeta>,
    );
}

pub(crate) trait RtcRtcpSendPort: Send + Sync {
    fn send_rtcp(&self, _payload: &[u8]);
}

#[derive(Default)]
pub(crate) struct NullRtcMediaSink;

impl RtcMediaSink for NullRtcMediaSink {
    fn on_raw_packet(
        &mut self,
        _packet: &RtcMediaIngressPacket,
        _route_label: RtcMediaRouteLabel,
        _route_reason: &str,
        _rtp_meta: Option<&RtcRtpPacketMeta>,
    ) {
    }
}

#[derive(Default)]
pub(crate) struct NullRtcRtcpSendPort;

impl RtcRtcpSendPort for NullRtcRtcpSendPort {
    fn send_rtcp(&self, _payload: &[u8]) {}
}
