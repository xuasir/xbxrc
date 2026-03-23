use super::RtcMediaService;
use crate::transport::rtc::stream::packet_router::{classify_packet, RtcMediaRouteLabel};
use crate::transport::rtc::stream::packet_types::{
    MediaPacketKind, RtcMediaIngressPacket, RtcMediaPacketSource, RtcRtpPacketMeta,
};
use crate::XbxEngineMediaRuntimeStats;
use std::sync::{Arc, Mutex};

#[test]
fn raw_packet_observation_updates_summary() {
    let mut service = RtcMediaService::default();
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    service.observe_raw_packet(MediaPacketKind::Rtp, &runtime_stats);
    let summary = runtime_stats
        .lock()
        .ok()
        .and_then(|stats| stats.latest_observation_summary.clone())
        .unwrap_or_default();
    assert!(summary.contains("route=unknown"));
    assert!(summary.contains("source=unknown"));
}

#[test]
fn ingress_packet_updates_counts_identity_and_route_stats() {
    let mut service = RtcMediaService::default();
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));

    service.observe_ingress_packet(
        RtcMediaIngressPacket::new(
            MediaPacketKind::Rtp,
            1200,
            RtcMediaPacketSource::Track {
                track_id: "video-main".to_string(),
            },
        ),
        Some(RtcRtpPacketMeta {
            ssrc: 7,
            payload_type: 96,
            sequence_number: 15,
            timestamp: 1234,
            marker: true,
        }),
        &runtime_stats,
    );
    service.observe_ingress_packet(
        RtcMediaIngressPacket::new(MediaPacketKind::Rtcp, 128, RtcMediaPacketSource::Unknown),
        None,
        &runtime_stats,
    );

    let snapshot = service.snapshot();
    assert_eq!(snapshot.inbound_rtp_count, 1);
    assert_eq!(snapshot.inbound_rtp_bytes, 1200);
    assert_eq!(snapshot.inbound_rtcp_count, 1);
    assert_eq!(snapshot.inbound_rtcp_bytes, 128);
    assert_eq!(snapshot.inbound_primary_video_count, 1);
    assert_eq!(snapshot.inbound_primary_video_bytes, 1200);
    assert_eq!(snapshot.inbound_unknown_count, 1);
    assert_eq!(snapshot.inbound_unknown_bytes, 128);
    assert_eq!(snapshot.last_route_label, Some(RtcMediaRouteLabel::Unknown));
    let last_stream_identity = snapshot.last_stream_identity.clone().unwrap_or_default();
    assert_eq!(last_stream_identity.track_id, None);
}

#[test]
fn ingress_summary_contains_route_identity_and_rtp_meta() {
    let mut service = RtcMediaService::default();
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    service.observe_ingress_packet(
        RtcMediaIngressPacket::new(
            MediaPacketKind::Rtp,
            800,
            RtcMediaPacketSource::Track {
                track_id: "video-repair".to_string(),
            },
        ),
        Some(RtcRtpPacketMeta {
            ssrc: 111,
            payload_type: 120,
            sequence_number: 999,
            timestamp: 8888,
            marker: false,
        }),
        &runtime_stats,
    );
    let summary = runtime_stats
        .lock()
        .ok()
        .and_then(|stats| stats.latest_observation_summary.clone())
        .unwrap_or_default();
    assert!(summary.contains("route=repairVideo"));
    assert!(summary.contains("identity=track_id=video-repair"));
    assert!(summary.contains("source=track:video-repair"));
    assert!(summary.contains("ssrc=111"));
    assert!(summary.contains("pt=120"));
    assert!(summary.contains("seq=999"));
    assert!(summary.contains("marker=false"));
}

#[test]
fn route_classifier_uses_track_hints_before_payload_fallback() {
    let primary_packet = RtcMediaIngressPacket::new(
        MediaPacketKind::Rtp,
        640,
        RtcMediaPacketSource::Track {
            track_id: "video-main".to_string(),
        },
    );
    let primary_route = classify_packet(&primary_packet, None, None);
    assert_eq!(primary_route.label, RtcMediaRouteLabel::PrimaryVideo);

    let repair_packet = RtcMediaIngressPacket::new(
        MediaPacketKind::Rtp,
        640,
        RtcMediaPacketSource::Track {
            track_id: "video-repair".to_string(),
        },
    );
    let repair_route = classify_packet(&repair_packet, None, None);
    assert_eq!(repair_route.label, RtcMediaRouteLabel::RepairVideo);

    let audio_packet = RtcMediaIngressPacket::new(
        MediaPacketKind::Rtp,
        640,
        RtcMediaPacketSource::Track {
            track_id: "audio-main".to_string(),
        },
    )
    .with_rtp_meta(Some(&RtcRtpPacketMeta {
        ssrc: 42,
        payload_type: 102,
        sequence_number: 777,
        timestamp: 9999,
        marker: true,
    }));
    let audio_meta = RtcRtpPacketMeta {
        ssrc: 42,
        payload_type: 102,
        sequence_number: 777,
        timestamp: 9999,
        marker: true,
    };
    let audio_route = classify_packet(&audio_packet, Some(&audio_meta), None);
    assert_eq!(audio_route.label, RtcMediaRouteLabel::Audio);

    let unknown_route = classify_packet(
        &RtcMediaIngressPacket::new(MediaPacketKind::Rtcp, 64, RtcMediaPacketSource::Unknown),
        None,
        None,
    );
    assert_eq!(unknown_route.label, RtcMediaRouteLabel::Unknown);
    assert!(repair_route.reason.contains("route=repairVideo"));
}

#[test]
fn ingress_route_prefers_negotiated_answer_payload_map() {
    let mut service = RtcMediaService::default();
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    service.apply_remote_answer_sdp(concat!(
        "v=0\r\n",
        "m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n",
        "a=rtpmap:111 opus/48000/2\r\n",
        "m=video 9 UDP/TLS/RTP/SAVPF 124 97\r\n",
        "a=rtpmap:124 H264/90000\r\n",
        "a=rtpmap:97 rtx/90000\r\n",
        "a=fmtp:97 apt=124\r\n",
    ));
    service.observe_ingress_packet(
        RtcMediaIngressPacket::new(
            MediaPacketKind::Rtp,
            90,
            RtcMediaPacketSource::Track {
                track_id: "track:a02600fa-4386-4dbd-9369-bc62985855a0".to_string(),
            },
        ),
        Some(RtcRtpPacketMeta {
            ssrc: 870530164,
            payload_type: 111,
            sequence_number: 1,
            timestamp: 1000,
            marker: false,
        }),
        &runtime_stats,
    );

    let snapshot = service.snapshot();
    assert_eq!(snapshot.inbound_audio_count, 1);
    assert_eq!(snapshot.inbound_primary_video_count, 0);
    assert_eq!(snapshot.last_route_label, Some(RtcMediaRouteLabel::Audio));
}

#[test]
fn ingress_route_marks_red_and_flexfec_as_repair_under_answer_map() {
    let mut service = RtcMediaService::default();
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    service.apply_remote_answer_sdp(concat!(
        "v=0\r\n",
        "m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n",
        "a=rtpmap:111 opus/48000/2\r\n",
        "m=video 9 UDP/TLS/RTP/SAVPF 124 97 125 122\r\n",
        "a=rtpmap:124 H264/90000\r\n",
        "a=rtcp-fb:124 nack\r\n",
        "a=rtcp-fb:124 nack pli\r\n",
        "a=rtcp-fb:124 ccm fir\r\n",
        "a=rtcp-fb:124 transport-cc\r\n",
        "a=rtpmap:97 rtx/90000\r\n",
        "a=fmtp:97 apt=124\r\n",
        "a=rtpmap:125 red/90000\r\n",
        "a=rtpmap:122 flexfec-03/90000\r\n",
    ));

    service.observe_ingress_packet(
        RtcMediaIngressPacket::new(
            MediaPacketKind::Rtp,
            160,
            RtcMediaPacketSource::Track {
                track_id: "track:browser-video".to_string(),
            },
        ),
        Some(RtcRtpPacketMeta {
            ssrc: 7,
            payload_type: 125,
            sequence_number: 2,
            timestamp: 1200,
            marker: false,
        }),
        &runtime_stats,
    );
    service.observe_ingress_packet(
        RtcMediaIngressPacket::new(
            MediaPacketKind::Rtp,
            120,
            RtcMediaPacketSource::Track {
                track_id: "track:browser-video".to_string(),
            },
        ),
        Some(RtcRtpPacketMeta {
            ssrc: 7,
            payload_type: 122,
            sequence_number: 3,
            timestamp: 1400,
            marker: false,
        }),
        &runtime_stats,
    );

    let snapshot = service.snapshot();
    assert_eq!(snapshot.inbound_repair_video_count, 2);
    assert_eq!(snapshot.inbound_primary_video_count, 0);
    assert_eq!(snapshot.inbound_audio_count, 0);
    assert_eq!(
        snapshot.last_route_label,
        Some(RtcMediaRouteLabel::RepairVideo)
    );
}

#[test]
fn snapshot_keeps_last_packet_metadata() {
    let mut service = RtcMediaService::default();
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let packet = RtcMediaIngressPacket::new(
        MediaPacketKind::Rtp,
        640,
        RtcMediaPacketSource::Track {
            track_id: "video-main".to_string(),
        },
    );
    let meta = RtcRtpPacketMeta {
        ssrc: 42,
        payload_type: 102,
        sequence_number: 777,
        timestamp: 9999,
        marker: true,
    };
    service.observe_ingress_packet(packet.clone(), Some(meta.clone()), &runtime_stats);
    let snapshot = service.snapshot();
    assert_eq!(snapshot.last_packet_kind, Some(MediaPacketKind::Rtp));
    assert_eq!(snapshot.last_packet_source, Some(packet.source));
    assert_eq!(snapshot.last_rtp_meta, Some(meta));
    let last_stream_identity = snapshot.last_stream_identity.clone().unwrap_or_default();
    assert_eq!(
        last_stream_identity.track_id,
        Some("video-main".to_string())
    );
    assert_eq!(last_stream_identity.ssrc, Some(42));
}
