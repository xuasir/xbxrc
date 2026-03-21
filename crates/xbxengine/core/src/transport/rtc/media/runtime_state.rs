use crate::transport::rtc::media::packet_router::{RtcMediaRouteDecision, RtcMediaRouteLabel};
use crate::transport::rtc::media::packet_types::{
    MediaPacketKind, RtcMediaIngressPacket, RtcMediaPacketSource, RtcMediaStreamIdentity,
    RtcRtpPacketMeta,
};

#[derive(Clone, Debug, Default)]
pub(crate) struct RtcMediaRuntimeState {
    pub(crate) inbound_rtp_count: u64,
    pub(crate) inbound_rtcp_count: u64,
    pub(crate) inbound_rtp_bytes: u64,
    pub(crate) inbound_rtcp_bytes: u64,
    pub(crate) inbound_primary_video_count: u64,
    pub(crate) inbound_primary_video_bytes: u64,
    pub(crate) inbound_repair_video_count: u64,
    pub(crate) inbound_repair_video_bytes: u64,
    pub(crate) inbound_audio_count: u64,
    pub(crate) inbound_audio_bytes: u64,
    pub(crate) inbound_unknown_count: u64,
    pub(crate) inbound_unknown_bytes: u64,
    pub(crate) last_packet_kind: Option<MediaPacketKind>,
    pub(crate) last_packet_source: Option<RtcMediaPacketSource>,
    pub(crate) last_stream_identity: Option<RtcMediaStreamIdentity>,
    pub(crate) last_route_label: Option<RtcMediaRouteLabel>,
    pub(crate) last_route_reason: Option<String>,
    pub(crate) last_rtp_meta: Option<RtcRtpPacketMeta>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RtcMediaIngressSnapshot {
    pub(crate) inbound_rtp_count: u64,
    pub(crate) inbound_rtcp_count: u64,
    pub(crate) inbound_rtp_bytes: u64,
    pub(crate) inbound_rtcp_bytes: u64,
    pub(crate) inbound_primary_video_count: u64,
    pub(crate) inbound_primary_video_bytes: u64,
    pub(crate) inbound_repair_video_count: u64,
    pub(crate) inbound_repair_video_bytes: u64,
    pub(crate) inbound_audio_count: u64,
    pub(crate) inbound_audio_bytes: u64,
    pub(crate) inbound_unknown_count: u64,
    pub(crate) inbound_unknown_bytes: u64,
    pub(crate) last_packet_kind: Option<MediaPacketKind>,
    pub(crate) last_packet_source: Option<RtcMediaPacketSource>,
    pub(crate) last_stream_identity: Option<RtcMediaStreamIdentity>,
    pub(crate) last_route_label: Option<RtcMediaRouteLabel>,
    pub(crate) last_route_reason: Option<String>,
    pub(crate) last_rtp_meta: Option<RtcRtpPacketMeta>,
}

impl From<&RtcMediaRuntimeState> for RtcMediaIngressSnapshot {
    fn from(state: &RtcMediaRuntimeState) -> Self {
        Self {
            inbound_rtp_count: state.inbound_rtp_count,
            inbound_rtcp_count: state.inbound_rtcp_count,
            inbound_rtp_bytes: state.inbound_rtp_bytes,
            inbound_rtcp_bytes: state.inbound_rtcp_bytes,
            inbound_primary_video_count: state.inbound_primary_video_count,
            inbound_primary_video_bytes: state.inbound_primary_video_bytes,
            inbound_repair_video_count: state.inbound_repair_video_count,
            inbound_repair_video_bytes: state.inbound_repair_video_bytes,
            inbound_audio_count: state.inbound_audio_count,
            inbound_audio_bytes: state.inbound_audio_bytes,
            inbound_unknown_count: state.inbound_unknown_count,
            inbound_unknown_bytes: state.inbound_unknown_bytes,
            last_packet_kind: state.last_packet_kind,
            last_packet_source: state.last_packet_source.clone(),
            last_stream_identity: state.last_stream_identity.clone(),
            last_route_label: state.last_route_label,
            last_route_reason: state.last_route_reason.clone(),
            last_rtp_meta: state.last_rtp_meta.clone(),
        }
    }
}

impl RtcMediaRuntimeState {
    pub(crate) fn record_ingress(
        &mut self,
        packet: &RtcMediaIngressPacket,
        route: &RtcMediaRouteDecision,
        rtp_meta: Option<&RtcRtpPacketMeta>,
    ) {
        match packet.kind {
            MediaPacketKind::Rtp => {
                self.inbound_rtp_count = self.inbound_rtp_count.saturating_add(1);
                self.inbound_rtp_bytes = self
                    .inbound_rtp_bytes
                    .saturating_add(packet.byte_len as u64);
            }
            MediaPacketKind::Rtcp => {
                self.inbound_rtcp_count = self.inbound_rtcp_count.saturating_add(1);
                self.inbound_rtcp_bytes = self
                    .inbound_rtcp_bytes
                    .saturating_add(packet.byte_len as u64);
            }
        }
        match route.label {
            RtcMediaRouteLabel::PrimaryVideo => {
                self.inbound_primary_video_count =
                    self.inbound_primary_video_count.saturating_add(1);
                self.inbound_primary_video_bytes = self
                    .inbound_primary_video_bytes
                    .saturating_add(packet.byte_len as u64);
            }
            RtcMediaRouteLabel::RepairVideo => {
                self.inbound_repair_video_count = self.inbound_repair_video_count.saturating_add(1);
                self.inbound_repair_video_bytes = self
                    .inbound_repair_video_bytes
                    .saturating_add(packet.byte_len as u64);
            }
            RtcMediaRouteLabel::Audio => {
                self.inbound_audio_count = self.inbound_audio_count.saturating_add(1);
                self.inbound_audio_bytes = self
                    .inbound_audio_bytes
                    .saturating_add(packet.byte_len as u64);
            }
            RtcMediaRouteLabel::Unknown => {
                self.inbound_unknown_count = self.inbound_unknown_count.saturating_add(1);
                self.inbound_unknown_bytes = self
                    .inbound_unknown_bytes
                    .saturating_add(packet.byte_len as u64);
            }
        }
        self.last_packet_kind = Some(packet.kind);
        self.last_packet_source = Some(packet.source.clone());
        self.last_stream_identity = Some(packet.stream_identity.clone());
        self.last_route_label = Some(route.label);
        self.last_route_reason = Some(route.reason.clone());
        self.last_rtp_meta = rtp_meta.cloned();
    }
}
