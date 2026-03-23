use crate::transport::rtc::stream::packet_router::RtcMediaRouteLabel;
use crate::transport::rtc::stream::packet_types::{
    RtcAudioRtpPacket, RtcMediaIngressPacket, RtcRtpPacketMeta,
};
use crate::transport::rtc::stream::sink::RtcMediaSink;

pub(crate) struct RtcAudioPlaybackSink {
    tx: Option<tokio::sync::mpsc::Sender<RtcAudioRtpPacket>>,
}

impl RtcAudioPlaybackSink {
    pub(crate) fn new(tx: tokio::sync::mpsc::Sender<RtcAudioRtpPacket>) -> Self {
        Self { tx: Some(tx) }
    }

    pub(crate) fn disabled() -> Self {
        Self { tx: None }
    }
}

impl RtcMediaSink for RtcAudioPlaybackSink {
    fn on_raw_packet(
        &mut self,
        packet: &RtcMediaIngressPacket,
        route_label: RtcMediaRouteLabel,
        _route_reason: &str,
        _rtp_meta: Option<&RtcRtpPacketMeta>,
    ) {
        if !matches!(route_label, RtcMediaRouteLabel::Audio) {
            return;
        }
        let (Some(tx), Some(payload)) = (self.tx.as_ref(), packet.rtp_payload.as_ref()) else {
            return;
        };
        if let Err(err) = tx.try_send(RtcAudioRtpPacket {
            payload: payload.clone(),
        }) {
            crate::xbx_log_warn!(
                "[xbxengine][rtc][audio] playback sink ingress dropped err={}",
                err
            );
        }
    }
}
