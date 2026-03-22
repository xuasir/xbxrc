use crate::transport::rtc::media::packet_types::{
    MediaPacketKind, RtcMediaIngressPacket, RtcMediaPacketSource, RtcRtpPacketMeta,
};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RtcMediaRouteLabel {
    PrimaryVideo,
    RepairVideo,
    Audio,
    Unknown,
}

impl RtcMediaRouteLabel {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::PrimaryVideo => "primaryVideo",
            Self::RepairVideo => "repairVideo",
            Self::Audio => "audio",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RtcMediaRouteDecision {
    pub(crate) label: RtcMediaRouteLabel,
    pub(crate) reason: String,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RtcPayloadRouteMap {
    audio_payload_types: HashSet<u8>,
    primary_video_payload_types: HashSet<u8>,
    repair_video_payload_types: HashSet<u8>,
    repair_rtx_payload_types: HashSet<u8>,
    repair_rtx_apt_targets: HashMap<u8, u8>,
}

impl RtcPayloadRouteMap {
    pub(crate) fn classify_payload_type(&self, payload_type: u8) -> Option<RtcMediaRouteLabel> {
        if self.audio_payload_types.contains(&payload_type) {
            Some(RtcMediaRouteLabel::Audio)
        } else if self.repair_video_payload_types.contains(&payload_type) {
            Some(RtcMediaRouteLabel::RepairVideo)
        } else if self.primary_video_payload_types.contains(&payload_type) {
            Some(RtcMediaRouteLabel::PrimaryVideo)
        } else {
            None
        }
    }

    pub(crate) fn is_rtx_payload_type(&self, payload_type: u8) -> bool {
        self.repair_rtx_payload_types.contains(&payload_type)
    }

    pub(crate) fn primary_payload_type_for_rtx(&self, payload_type: u8) -> Option<u8> {
        self.repair_rtx_apt_targets.get(&payload_type).copied()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SdpMediaKind {
    Audio,
    Video,
    Other,
}

#[derive(Clone, Debug)]
struct SdpMediaSection {
    kind: SdpMediaKind,
    payload_types: Vec<u8>,
}

pub(crate) fn classify_packet(
    packet: &RtcMediaIngressPacket,
    rtp_meta: Option<&RtcRtpPacketMeta>,
    payload_route_map: Option<&RtcPayloadRouteMap>,
) -> RtcMediaRouteDecision {
    let track_hint = packet.stream_identity.track_hint();
    let lowered_hint = track_hint.map(|value| value.to_ascii_lowercase());
    let lowered_hint = lowered_hint.as_deref();

    let (label, rule) = if let Some(label) = classify_from_payload_map(rtp_meta, payload_route_map)
    {
        let rule = match label {
            RtcMediaRouteLabel::Audio => "sdp_pt_audio",
            RtcMediaRouteLabel::RepairVideo => "sdp_pt_repair_video",
            RtcMediaRouteLabel::PrimaryVideo => "sdp_pt_primary_video",
            RtcMediaRouteLabel::Unknown => "sdp_pt_unknown",
        };
        (label, rule)
    } else if is_audio_track_hint(lowered_hint) {
        (RtcMediaRouteLabel::Audio, "track_hint_audio")
    } else if is_repair_track_hint(lowered_hint) {
        (RtcMediaRouteLabel::RepairVideo, "track_hint_repair")
    } else if is_video_track_hint(lowered_hint) {
        (RtcMediaRouteLabel::PrimaryVideo, "track_hint_video")
    } else if let Some(meta) = rtp_meta {
        classify_rtp_meta(packet, meta)
    } else if matches!(packet.kind, MediaPacketKind::Rtp)
        && packet.stream_identity.track_hint().is_some()
    {
        (RtcMediaRouteLabel::PrimaryVideo, "rtp_default_primary")
    } else {
        (RtcMediaRouteLabel::Unknown, "unknown")
    };

    let reason = build_route_reason(packet, label, rule, track_hint, rtp_meta);
    RtcMediaRouteDecision { label, reason }
}

fn classify_from_payload_map(
    rtp_meta: Option<&RtcRtpPacketMeta>,
    payload_route_map: Option<&RtcPayloadRouteMap>,
) -> Option<RtcMediaRouteLabel> {
    let payload_type = rtp_meta.map(|meta| meta.payload_type)?;
    payload_route_map?.classify_payload_type(payload_type)
}

fn build_route_reason(
    packet: &RtcMediaIngressPacket,
    label: RtcMediaRouteLabel,
    rule: &str,
    track_hint: Option<&str>,
    rtp_meta: Option<&RtcRtpPacketMeta>,
) -> String {
    let packet_kind = match packet.kind {
        MediaPacketKind::Rtp => "rtp",
        MediaPacketKind::Rtcp => "rtcp",
    };
    let source = match &packet.source {
        RtcMediaPacketSource::Unknown => "source=unknown".to_string(),
        RtcMediaPacketSource::Track { .. } => format!(
            "source=track:{}",
            track_hint.unwrap_or(packet.stream_identity.track_hint().unwrap_or("-"))
        ),
    };
    let meta_summary = rtp_meta
        .map(|meta| {
            format!(
                "ssrc={} pt={} seq={} marker={}",
                meta.ssrc, meta.payload_type, meta.sequence_number, meta.marker
            )
        })
        .unwrap_or_else(|| "ssrc=- pt=- seq=- marker=-".to_string());
    format!(
        "rule={rule} {source} {meta_summary} kind={packet_kind} route={}",
        label.as_str()
    )
}

fn is_audio_track_hint(track_hint: Option<&str>) -> bool {
    track_hint.is_some_and(|value| {
        value.contains("audio") || value.contains("mic") || value.contains("voice")
    })
}

fn is_repair_track_hint(track_hint: Option<&str>) -> bool {
    track_hint.is_some_and(|value| {
        value.contains("repair")
            || value.contains("rtx")
            || value.contains("fec")
            || value.contains("recovery")
            || value.contains("redundant")
    })
}

fn is_video_track_hint(track_hint: Option<&str>) -> bool {
    track_hint.is_some_and(|value| value.contains("video"))
}

fn classify_rtp_meta(
    packet: &RtcMediaIngressPacket,
    meta: &RtcRtpPacketMeta,
) -> (RtcMediaRouteLabel, &'static str) {
    if is_probable_audio_payload_type(meta.payload_type) {
        (RtcMediaRouteLabel::Audio, "audio_payload_type")
    } else if matches!(packet.kind, MediaPacketKind::Rtp)
        && packet.stream_identity.track_hint().is_some()
    {
        (
            RtcMediaRouteLabel::PrimaryVideo,
            "rtp_track_default_primary",
        )
    } else {
        (RtcMediaRouteLabel::Unknown, "rtp_meta_unknown")
    }
}

fn is_probable_audio_payload_type(payload_type: u8) -> bool {
    matches!(payload_type, 0 | 3 | 8 | 9 | 13 | 18 | 19 | 101 | 102 | 103)
}

pub(crate) fn parse_payload_route_map_from_answer(answer_sdp: &str) -> Option<RtcPayloadRouteMap> {
    let mut sections = Vec::<SdpMediaSection>::new();
    let mut current_section_index = None::<usize>;
    let mut codec_by_payload_type = HashMap::<u8, String>::new();
    let mut rtx_apt_targets = HashMap::<u8, u8>::new();

    for raw_line in answer_sdp.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(section) = parse_media_section(line) {
            sections.push(section);
            current_section_index = Some(sections.len().saturating_sub(1));
            continue;
        }
        let Some(section_index) = current_section_index else {
            continue;
        };
        if let Some((payload_type, codec)) = parse_rtpmap_line(line) {
            if sections[section_index]
                .payload_types
                .contains(&payload_type)
            {
                codec_by_payload_type.insert(payload_type, codec);
            }
        }
        if let Some((payload_type, apt_payload_type)) = parse_apt_line(line) {
            if sections[section_index]
                .payload_types
                .contains(&payload_type)
            {
                rtx_apt_targets.insert(payload_type, apt_payload_type);
            }
        }
    }

    let mut route_map = RtcPayloadRouteMap::default();
    for section in sections {
        match section.kind {
            SdpMediaKind::Audio => {
                route_map.audio_payload_types.extend(section.payload_types);
            }
            SdpMediaKind::Video => {
                for payload_type in section.payload_types {
                    let codec = codec_by_payload_type
                        .get(&payload_type)
                        .map(|value| value.as_str())
                        .unwrap_or_default();
                    if is_repair_codec(codec) {
                        route_map.repair_video_payload_types.insert(payload_type);
                        if codec == "rtx" {
                            route_map.repair_rtx_payload_types.insert(payload_type);
                            if let Some(&apt_payload_type) = rtx_apt_targets.get(&payload_type) {
                                route_map.repair_rtx_apt_targets.insert(payload_type, apt_payload_type);
                            }
                        }
                    } else if rtx_apt_targets.values().any(|&apt| apt == payload_type)
                        || !codec.is_empty()
                    {
                        route_map.primary_video_payload_types.insert(payload_type);
                    } else {
                        // 视频 m-line 中没有 rtpmap 的动态 PT，默认按主视频处理，避免误判成 unknown。
                        route_map.primary_video_payload_types.insert(payload_type);
                    }
                }
            }
            SdpMediaKind::Other => {}
        }
    }

    if route_map.audio_payload_types.is_empty()
        && route_map.primary_video_payload_types.is_empty()
        && route_map.repair_video_payload_types.is_empty()
    {
        None
    } else {
        Some(route_map)
    }
}

fn parse_media_section(line: &str) -> Option<SdpMediaSection> {
    let value = line.strip_prefix("m=")?;
    let mut tokens = value.split_whitespace();
    let media_kind = match tokens.next()? {
        "audio" => SdpMediaKind::Audio,
        "video" => SdpMediaKind::Video,
        _ => SdpMediaKind::Other,
    };
    let payload_types = tokens
        .skip(2)
        .filter_map(|token| token.parse::<u8>().ok())
        .collect::<Vec<_>>();
    Some(SdpMediaSection {
        kind: media_kind,
        payload_types,
    })
}

fn parse_rtpmap_line(line: &str) -> Option<(u8, String)> {
    let value = line.strip_prefix("a=rtpmap:")?;
    let mut tokens = value.split_whitespace();
    let payload_type = tokens.next()?.parse::<u8>().ok()?;
    let codec = tokens
        .next()?
        .split('/')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if codec.is_empty() {
        None
    } else {
        Some((payload_type, codec))
    }
}

fn parse_apt_line(line: &str) -> Option<(u8, u8)> {
    let value = line.strip_prefix("a=fmtp:")?;
    let mut tokens = value.split_whitespace();
    let payload_type = tokens.next()?.parse::<u8>().ok()?;
    let fmtp = tokens.collect::<Vec<_>>().join(" ");
    let apt_value = fmtp.split(';').find_map(|entry| {
        let mut kv = entry.trim().splitn(2, '=');
        let key = kv.next()?.trim();
        let value = kv.next()?.trim();
        if key.eq_ignore_ascii_case("apt") {
            value.parse::<u8>().ok()
        } else {
            None
        }
    })?;
    Some((payload_type, apt_value))
}

fn is_repair_codec(codec: &str) -> bool {
    matches!(codec, "rtx" | "red" | "ulpfec" | "flexfec" | "flexfec-03")
}

#[cfg(test)]
mod tests {
    use super::{parse_payload_route_map_from_answer, RtcMediaRouteLabel};

    #[test]
    fn answer_payload_map_marks_audio_primary_and_repair_video() {
        let answer = concat!(
            "v=0\r\n",
            "m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n",
            "a=rtpmap:111 opus/48000/2\r\n",
            "m=video 9 UDP/TLS/RTP/SAVPF 124 97 116\r\n",
            "a=rtpmap:124 H264/90000\r\n",
            "a=rtpmap:97 rtx/90000\r\n",
            "a=fmtp:97 apt=124\r\n",
            "a=rtpmap:116 ulpfec/90000\r\n",
        );
        let map = parse_payload_route_map_from_answer(answer).expect("route map should exist");
        assert_eq!(
            map.classify_payload_type(111),
            Some(RtcMediaRouteLabel::Audio)
        );
        assert_eq!(
            map.classify_payload_type(124),
            Some(RtcMediaRouteLabel::PrimaryVideo)
        );
        assert_eq!(
            map.classify_payload_type(97),
            Some(RtcMediaRouteLabel::RepairVideo)
        );
        assert_eq!(
            map.classify_payload_type(116),
            Some(RtcMediaRouteLabel::RepairVideo)
        );
    }

    #[test]
    fn answer_payload_map_supports_browser_h264_transportcc_and_repair_family() {
        let answer = concat!(
            "v=0\r\n",
            "m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n",
            "a=rtpmap:111 opus/48000/2\r\n",
            "m=video 9 UDP/TLS/RTP/SAVPF 124 97 125 116 122\r\n",
            "a=rtpmap:124 H264/90000\r\n",
            "a=rtcp-fb:124 nack\r\n",
            "a=rtcp-fb:124 nack pli\r\n",
            "a=rtcp-fb:124 ccm fir\r\n",
            "a=rtcp-fb:124 transport-cc\r\n",
            "a=rtpmap:97 rtx/90000\r\n",
            "a=fmtp:97 apt=124\r\n",
            "a=rtpmap:125 red/90000\r\n",
            "a=rtpmap:116 ulpfec/90000\r\n",
            "a=rtpmap:122 flexfec-03/90000\r\n",
        );
        let map = parse_payload_route_map_from_answer(answer).expect("route map should exist");
        assert_eq!(
            map.classify_payload_type(111),
            Some(RtcMediaRouteLabel::Audio)
        );
        assert_eq!(
            map.classify_payload_type(124),
            Some(RtcMediaRouteLabel::PrimaryVideo)
        );
        assert_eq!(
            map.classify_payload_type(97),
            Some(RtcMediaRouteLabel::RepairVideo)
        );
        assert_eq!(
            map.classify_payload_type(125),
            Some(RtcMediaRouteLabel::RepairVideo)
        );
        assert_eq!(
            map.classify_payload_type(116),
            Some(RtcMediaRouteLabel::RepairVideo)
        );
        assert_eq!(
            map.classify_payload_type(122),
            Some(RtcMediaRouteLabel::RepairVideo)
        );
    }
}
