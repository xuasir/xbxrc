use crate::transport::rtc::media::packet_router::RtcMediaRouteLabel;
use crate::transport::rtc::media::packet_types::{
    RtcMediaIngressPacket, RtcRtpPacketMeta, RtcVideoRtpPacket,
};
use crate::transport::rtc::media::sink::RtcMediaSink;

pub(crate) struct RtcVideoSourceSink {
    pub(crate) tx: tokio::sync::mpsc::Sender<RtcVideoRtpPacket>,
}

impl RtcMediaSink for RtcVideoSourceSink {
    fn on_raw_packet(
        &mut self,
        packet: &RtcMediaIngressPacket,
        route_label: RtcMediaRouteLabel,
        _route_reason: &str,
        rtp_meta: Option<&RtcRtpPacketMeta>,
    ) {
        if !matches!(route_label, RtcMediaRouteLabel::PrimaryVideo) {
            return;
        }
        let (Some(meta), Some(payload)) = (rtp_meta, packet.rtp_payload.as_ref()) else {
            return;
        };
        if let Err(err) = self.tx.try_send(RtcVideoRtpPacket {
            payload: payload.clone(),
            meta: meta.clone(),
        }) {
            crate::xbx_log_warn!(
                "[xbxengine][rtc] video source sink ingress dropped err={}",
                err
            );
        }
    }
}
