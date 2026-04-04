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
#[path = "sink.test.rs"]
mod tests;
