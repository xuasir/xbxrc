use crate::transport::rtc::media::packet_router::{RtcMediaRouteLabel, RtcPayloadRouteMap};
use crate::transport::rtc::media::packet_types::{
    RtcMediaIngressPacket, RtcRtpPacketMeta, RtcVideoRtpPacket,
};
use crate::transport::rtc::media::sink::RtcMediaSink;

pub(crate) struct RtcVideoSourceSink {
    pub(crate) tx: tokio::sync::mpsc::Sender<RtcVideoRtpPacket>,
    pub(super) payload_route_map: Option<RtcPayloadRouteMap>,
}

impl RtcMediaSink for RtcVideoSourceSink {
    fn apply_payload_route_map(&mut self, payload_route_map: Option<RtcPayloadRouteMap>) {
        self.payload_route_map = payload_route_map;
    }

    fn on_raw_packet(
        &mut self,
        packet: &RtcMediaIngressPacket,
        route_label: RtcMediaRouteLabel,
        _route_reason: &str,
        rtp_meta: Option<&RtcRtpPacketMeta>,
    ) {
        let Some((meta, payload)) = normalize_video_packet(
            packet,
            route_label,
            rtp_meta,
            self.payload_route_map.as_ref(),
        ) else {
            return;
        };
        if let Err(err) = self.tx.try_send(RtcVideoRtpPacket { payload, meta }) {
            crate::xbx_log_warn!(
                "[xbxengine][rtc] video source sink ingress dropped err={}",
                err
            );
        }
    }
}

fn normalize_video_packet(
    packet: &RtcMediaIngressPacket,
    route_label: RtcMediaRouteLabel,
    rtp_meta: Option<&RtcRtpPacketMeta>,
    payload_route_map: Option<&RtcPayloadRouteMap>,
) -> Option<(RtcRtpPacketMeta, Vec<u8>)> {
    let (Some(meta), Some(payload)) = (rtp_meta, packet.rtp_payload.as_ref()) else {
        return None;
    };

    match route_label {
        RtcMediaRouteLabel::PrimaryVideo => Some((meta.clone(), payload.clone())),
        RtcMediaRouteLabel::RepairVideo => {
            if !is_rtx_payload(meta.payload_type, payload_route_map) {
                if meta.payload_type == 97 {
                    crate::xbx_log_debug!(
                        "[RtcVideoSourceSink] ignoring non-RTX repair payload pt={} len={}",
                        meta.payload_type,
                        payload.len()
                    );
                }
                return None;
            }
            unpack_rtx_packet(meta, payload, payload_route_map)
        }
        _ => None,
    }
}

fn is_rtx_payload(payload_type: u8, payload_route_map: Option<&RtcPayloadRouteMap>) -> bool {
    payload_route_map
        .map(|map| map.is_rtx_payload_type(payload_type))
        .unwrap_or(payload_type == 97)
}

fn unpack_rtx_packet(
    meta: &RtcRtpPacketMeta,
    payload: &[u8],
    payload_route_map: Option<&RtcPayloadRouteMap>,
) -> Option<(RtcRtpPacketMeta, Vec<u8>)> {
    if payload.len() < 2 {
        return None;
    }
    let original_sequence = u16::from_be_bytes([payload[0], payload[1]]);
    let mut normalized_meta = meta.clone();
    normalized_meta.sequence_number = original_sequence;
    if let Some(primary_payload_type) = payload_route_map
        .and_then(|map| map.primary_payload_type_for_rtx(meta.payload_type))
    {
        normalized_meta.payload_type = primary_payload_type;
    }
    Some((normalized_meta, payload[2..].to_vec()))
}

#[cfg(test)]
mod tests {
    use super::RtcVideoSourceSink;
    use crate::transport::rtc::media::packet_router::parse_payload_route_map_from_answer;
    use crate::transport::rtc::media::packet_router::RtcMediaRouteLabel;
    use crate::transport::rtc::media::packet_types::{
        MediaPacketKind, RtcMediaIngressPacket, RtcMediaPacketSource, RtcRtpPacketMeta,
    };
    use crate::transport::rtc::media::sink::RtcMediaSink;
    use std::time::Duration;

    #[tokio::test]
    async fn repair_rtx_packet_is_unpacked_and_reinjected_as_primary_video() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let mut sink = RtcVideoSourceSink {
            tx,
            payload_route_map: parse_payload_route_map_from_answer(concat!(
                "v=0\r\n",
                "m=video 9 UDP/TLS/RTP/SAVPF 124 97 116\r\n",
                "a=rtpmap:124 H264/90000\r\n",
                "a=rtpmap:97 rtx/90000\r\n",
                "a=fmtp:97 apt=124\r\n",
                "a=rtpmap:116 ulpfec/90000\r\n",
            )),
        };
        let packet = RtcMediaIngressPacket::new(
            MediaPacketKind::Rtp,
            6,
            RtcMediaPacketSource::Track {
                track_id: "video-repair".to_string(),
            },
        )
        .with_rtp_payload(vec![0x12, 0x34, 0xAA, 0xBB]);
        let meta = RtcRtpPacketMeta {
            ssrc: 99,
            payload_type: 97,
            sequence_number: 9000,
            timestamp: 123456,
            marker: true,
        };
        sink.on_raw_packet(
            &packet,
            RtcMediaRouteLabel::RepairVideo,
            "route=repairVideo",
            Some(&meta),
        );

        let normalized = tokio::time::timeout(Duration::from_millis(50), rx.recv())
            .await
            .expect("packet should be reinjected")
            .expect("normalized packet should exist");
        assert_eq!(normalized.meta.sequence_number, 0x1234);
        assert_eq!(normalized.meta.payload_type, 124);
        assert_eq!(normalized.meta.ssrc, 99);
        assert_eq!(normalized.payload, vec![0xAA, 0xBB]);
    }
}
