use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::stream::packet_router::{RtcMediaRouteLabel, RtcPayloadRouteMap};
use crate::transport::rtc::stream::packet_types::{
    RtcMediaIngressPacket, RtcRtpPacketMeta, RtcVideoIngressKind, RtcVideoRepairMetadata,
    RtcVideoRtpPacket,
};
use crate::transport::rtc::stream::sink::RtcMediaSink;
use crate::XbxEngineVideoRtxReinjectObservation;

pub(crate) struct RtcVideoSourceSink {
    pub(crate) tx: tokio::sync::mpsc::Sender<RtcVideoRtpPacket>,
    pub(super) payload_route_map: Option<RtcPayloadRouteMap>,
    pub(super) runtime_stats: RuntimeStatsSink,
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
        let Some(normalized) = normalize_video_packet(
            packet,
            route_label,
            rtp_meta,
            self.payload_route_map.as_ref(),
        ) else {
            return;
        };
        if let Err(err) = self.tx.try_send(normalized.clone()) {
            crate::xbx_log_warn!(
                "[xbxengine][rtc] video source sink ingress dropped err={}",
                err
            );
            return;
        }
        if let Some(observation) = build_reinject_queued_observation(&normalized) {
            self.runtime_stats.record_video_rtx_reinject(observation);
        }
    }
}

fn normalize_video_packet(
    packet: &RtcMediaIngressPacket,
    route_label: RtcMediaRouteLabel,
    rtp_meta: Option<&RtcRtpPacketMeta>,
    payload_route_map: Option<&RtcPayloadRouteMap>,
) -> Option<RtcVideoRtpPacket> {
    let (Some(meta), Some(payload)) = (rtp_meta, packet.rtp_payload.as_ref()) else {
        return None;
    };

    match route_label {
        RtcMediaRouteLabel::PrimaryVideo => Some(RtcVideoRtpPacket {
            payload: payload.clone(),
            meta: meta.clone(),
            ingress_kind: RtcVideoIngressKind::Primary,
        }),
        RtcMediaRouteLabel::RepairVideo => {
            normalize_repair_video_packet(meta, payload, payload_route_map)
        }
        _ => None,
    }
}

fn normalize_repair_video_packet(
    meta: &RtcRtpPacketMeta,
    payload: &[u8],
    payload_route_map: Option<&RtcPayloadRouteMap>,
) -> Option<RtcVideoRtpPacket> {
    if is_rtx_payload(meta.payload_type, payload_route_map) {
        return unpack_rtx_packet(meta, payload, payload_route_map);
    }

    if is_primary_video_payload(meta.payload_type, payload_route_map) {
        let primary_ssrc = payload_route_map.and_then(|map| map.primary_ssrc_for_repair(meta.ssrc));
        let Some(primary_ssrc) = primary_ssrc else {
            crate::xbx_log_debug!(
                "[RtcVideoSourceSink] dropping repair-route primary payload without FID mapping pt={} ssrc={} seq={}",
                meta.payload_type,
                meta.ssrc,
                meta.sequence_number
            );
            return None;
        };
        crate::xbx_log_debug!(
            "[RtcVideoSourceSink] repair route carried primary video payload pt={} seq={}",
            meta.payload_type,
            meta.sequence_number
        );
        let mut normalized_meta = meta.clone();
        normalized_meta.ssrc = primary_ssrc;
        return Some(RtcVideoRtpPacket {
            payload: payload.to_vec(),
            meta: normalized_meta,
            ingress_kind: RtcVideoIngressKind::RepairPrimaryPassThrough {
                repair: repair_metadata(meta),
            },
        });
    }

    crate::xbx_log_debug!(
        "[RtcVideoSourceSink] ignoring unsupported repair payload pt={} len={}",
        meta.payload_type,
        payload.len()
    );
    None
}

fn is_rtx_payload(payload_type: u8, payload_route_map: Option<&RtcPayloadRouteMap>) -> bool {
    payload_route_map
        .map(|map| map.is_rtx_payload_type(payload_type))
        .unwrap_or(false)
}

fn is_primary_video_payload(
    payload_type: u8,
    payload_route_map: Option<&RtcPayloadRouteMap>,
) -> bool {
    payload_route_map
        .map(|map| map.is_primary_video_payload_type(payload_type))
        .unwrap_or(false)
}

fn unpack_rtx_packet(
    meta: &RtcRtpPacketMeta,
    payload: &[u8],
    payload_route_map: Option<&RtcPayloadRouteMap>,
) -> Option<RtcVideoRtpPacket> {
    if payload.len() < 2 {
        crate::xbx_log_debug!(
            "[RtcVideoSourceSink] truncated RTX payload pt={} seq={} len={}",
            meta.payload_type,
            meta.sequence_number,
            payload.len()
        );
        return None;
    }
    let original_sequence = u16::from_be_bytes([payload[0], payload[1]]);
    let mut normalized_meta = meta.clone();
    normalized_meta.sequence_number = original_sequence;
    let payload_route_map = payload_route_map?;
    let primary_payload_type = payload_route_map.primary_payload_type_for_rtx(meta.payload_type)?;
    let primary_ssrc = payload_route_map.primary_ssrc_for_repair(meta.ssrc)?;
    normalized_meta.payload_type = primary_payload_type;
    normalized_meta.ssrc = primary_ssrc;
    Some(RtcVideoRtpPacket {
        payload: payload[2..].to_vec(),
        meta: normalized_meta,
        ingress_kind: RtcVideoIngressKind::RtxReinject {
            repair: repair_metadata(meta),
        },
    })
}

fn repair_metadata(meta: &RtcRtpPacketMeta) -> RtcVideoRepairMetadata {
    RtcVideoRepairMetadata {
        native_ssrc: meta.ssrc,
        native_payload_type: meta.payload_type,
        native_sequence_number: meta.sequence_number,
    }
}

fn build_reinject_queued_observation(
    packet: &RtcVideoRtpPacket,
) -> Option<XbxEngineVideoRtxReinjectObservation> {
    let (repair, primary_ssrc) = match packet.ingress_kind {
        RtcVideoIngressKind::Primary => return None,
        RtcVideoIngressKind::RepairPrimaryPassThrough { repair } => (repair, packet.meta.ssrc),
        RtcVideoIngressKind::RtxReinject { repair } => (repair, packet.meta.ssrc),
    };
    Some(XbxEngineVideoRtxReinjectObservation {
        stage: "queued".to_string(),
        primary_ssrc,
        repair_ssrc: repair.native_ssrc,
        sequence_number: packet.meta.sequence_number,
        rtp_timestamp: packet.meta.timestamp,
        pending_queue_len: 1,
        native_sequence_number: Some(repair.native_sequence_number),
        matched_head_gap: false,
        matched_nack_range: false,
        matched_pending_gap: false,
        matched_gap_sequence: None,
        matched_nack_first_sequence: None,
        matched_nack_last_sequence: None,
        observed_at_ms: crate::transport::rtc::stream::video_source::now_ms_f64(),
    })
}

#[cfg(test)]
mod tests {
    use super::RtcVideoSourceSink;
    use crate::runtime_stats_sink::RuntimeStatsSink;
    use crate::transport::rtc::stream::packet_router::parse_payload_route_map_from_answer;
    use crate::transport::rtc::stream::packet_router::RtcMediaRouteLabel;
    use crate::transport::rtc::stream::packet_types::{
        MediaPacketKind, RtcMediaIngressPacket, RtcMediaPacketSource, RtcRtpPacketMeta,
        RtcVideoIngressKind,
    };
    use crate::transport::rtc::stream::sink::RtcMediaSink;
    use crate::XbxEngineMediaRuntimeStats;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    fn runtime_stats_sink() -> RuntimeStatsSink {
        RuntimeStatsSink::new(Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default())))
    }

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
                "a=ssrc-group:FID 1111 99\r\n",
            )),
            runtime_stats: runtime_stats_sink(),
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
        assert_eq!(normalized.meta.ssrc, 1111);
        assert_eq!(normalized.payload, vec![0xAA, 0xBB]);
        assert!(matches!(
            normalized.ingress_kind,
            RtcVideoIngressKind::RtxReinject { .. }
        ));
    }

    #[tokio::test]
    async fn primary_video_packet_passes_through_without_rewrite() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let mut sink = RtcVideoSourceSink {
            tx,
            payload_route_map: None,
            runtime_stats: runtime_stats_sink(),
        };
        let packet = RtcMediaIngressPacket::new(
            MediaPacketKind::Rtp,
            4,
            RtcMediaPacketSource::Track {
                track_id: "video-main".to_string(),
            },
        )
        .with_rtp_payload(vec![0x65, 0x88, 0x81, 0x00]);
        let meta = RtcRtpPacketMeta {
            ssrc: 7,
            payload_type: 124,
            sequence_number: 321,
            timestamp: 654321,
            marker: true,
        };

        sink.on_raw_packet(
            &packet,
            RtcMediaRouteLabel::PrimaryVideo,
            "route=primaryVideo",
            Some(&meta),
        );

        let normalized = tokio::time::timeout(Duration::from_millis(50), rx.recv())
            .await
            .expect("packet should pass through")
            .expect("normalized packet should exist");
        assert_eq!(normalized.meta, meta);
        assert_eq!(normalized.payload, vec![0x65, 0x88, 0x81, 0x00]);
        assert_eq!(normalized.ingress_kind, RtcVideoIngressKind::Primary);
    }

    #[tokio::test]
    async fn non_rtx_repair_payload_is_ignored() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let mut sink = RtcVideoSourceSink {
            tx,
            payload_route_map: parse_payload_route_map_from_answer(concat!(
                "v=0\r\n",
                "m=video 9 UDP/TLS/RTP/SAVPF 124 97\r\n",
                "a=rtpmap:124 H264/90000\r\n",
                "a=rtpmap:97 rtx/90000\r\n",
                "a=fmtp:97 apt=124\r\n",
                "a=ssrc-group:FID 1111 99\r\n",
            )),
            runtime_stats: runtime_stats_sink(),
        };
        let packet = RtcMediaIngressPacket::new(
            MediaPacketKind::Rtp,
            4,
            RtcMediaPacketSource::Track {
                track_id: "video-repair".to_string(),
            },
        )
        .with_rtp_payload(vec![0x00, 0x01, 0xAA, 0xBB]);
        let meta = RtcRtpPacketMeta {
            ssrc: 99,
            payload_type: 125,
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

        assert!(tokio::time::timeout(Duration::from_millis(30), rx.recv())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn repair_route_primary_payload_passes_through_without_drop() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let mut sink = RtcVideoSourceSink {
            tx,
            payload_route_map: parse_payload_route_map_from_answer(concat!(
                "v=0\r\n",
                "m=video 9 UDP/TLS/RTP/SAVPF 124 97\r\n",
                "a=rtpmap:124 H264/90000\r\n",
                "a=rtpmap:97 rtx/90000\r\n",
                "a=fmtp:97 apt=124\r\n",
                "a=ssrc-group:FID 1111 99\r\n",
            )),
            runtime_stats: runtime_stats_sink(),
        };
        let packet = RtcMediaIngressPacket::new(
            MediaPacketKind::Rtp,
            4,
            RtcMediaPacketSource::Track {
                track_id: "video-repair".to_string(),
            },
        )
        .with_rtp_payload(vec![0x65, 0x88, 0x81, 0x00]);
        let meta = RtcRtpPacketMeta {
            ssrc: 99,
            payload_type: 124,
            sequence_number: 9001,
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
            .expect("packet should pass through")
            .expect("normalized packet should exist");
        assert_eq!(normalized.meta.payload_type, meta.payload_type);
        assert_eq!(normalized.meta.sequence_number, meta.sequence_number);
        assert_eq!(normalized.meta.timestamp, meta.timestamp);
        assert_eq!(normalized.meta.marker, meta.marker);
        assert_eq!(normalized.meta.ssrc, 1111);
        assert_eq!(normalized.payload, vec![0x65, 0x88, 0x81, 0x00]);
        assert!(matches!(
            normalized.ingress_kind,
            RtcVideoIngressKind::RepairPrimaryPassThrough { .. }
        ));
    }

    #[tokio::test]
    async fn repair_rtx_packet_without_apt_mapping_is_ignored() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let mut sink = RtcVideoSourceSink {
            tx,
            payload_route_map: parse_payload_route_map_from_answer(concat!(
                "v=0\r\n",
                "m=video 9 UDP/TLS/RTP/SAVPF 124 97\r\n",
                "a=rtpmap:124 H264/90000\r\n",
                "a=rtpmap:97 rtx/90000\r\n",
                "a=ssrc-group:FID 1111 99\r\n",
            )),
            runtime_stats: runtime_stats_sink(),
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

        assert!(tokio::time::timeout(Duration::from_millis(30), rx.recv())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn repair_rtx_packet_without_fid_mapping_is_ignored() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let mut sink = RtcVideoSourceSink {
            tx,
            payload_route_map: parse_payload_route_map_from_answer(concat!(
                "v=0\r\n",
                "m=video 9 UDP/TLS/RTP/SAVPF 124 97\r\n",
                "a=rtpmap:124 H264/90000\r\n",
                "a=rtpmap:97 rtx/90000\r\n",
                "a=fmtp:97 apt=124\r\n",
            )),
            runtime_stats: runtime_stats_sink(),
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

        assert!(tokio::time::timeout(Duration::from_millis(30), rx.recv())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn truncated_rtx_payload_is_ignored() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let mut sink = RtcVideoSourceSink {
            tx,
            payload_route_map: parse_payload_route_map_from_answer(concat!(
                "v=0\r\n",
                "m=video 9 UDP/TLS/RTP/SAVPF 124 97\r\n",
                "a=rtpmap:124 H264/90000\r\n",
                "a=rtpmap:97 rtx/90000\r\n",
                "a=fmtp:97 apt=124\r\n",
            )),
            runtime_stats: runtime_stats_sink(),
        };
        let packet = RtcMediaIngressPacket::new(
            MediaPacketKind::Rtp,
            1,
            RtcMediaPacketSource::Track {
                track_id: "video-repair".to_string(),
            },
        )
        .with_rtp_payload(vec![0xAB]);
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

        assert!(tokio::time::timeout(Duration::from_millis(30), rx.recv())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn repair_rtx_without_payload_map_is_not_reinjected_by_pt_guess() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let mut sink = RtcVideoSourceSink {
            tx,
            payload_route_map: None,
            runtime_stats: runtime_stats_sink(),
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

        assert!(tokio::time::timeout(Duration::from_millis(30), rx.recv())
            .await
            .is_err());
    }
}
