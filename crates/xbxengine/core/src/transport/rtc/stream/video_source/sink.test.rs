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

fn runtime_stats_pair() -> (Arc<Mutex<XbxEngineMediaRuntimeStats>>, RuntimeStatsSink) {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());
    (runtime_stats, sink)
}

fn build_sink(
    tx: tokio::sync::mpsc::Sender<crate::transport::rtc::stream::packet_types::RtcVideoRtpPacket>,
) -> RtcVideoSourceSink {
    let (_, runtime_stats) = runtime_stats_pair();
    RtcVideoSourceSink::new(tx, runtime_stats)
}

#[tokio::test]
async fn repair_rtx_packet_is_unpacked_and_reinjected_as_primary_video() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let mut sink = build_sink(tx);
    sink.payload_route_map = parse_payload_route_map_from_answer(concat!(
        "v=0\r\n",
        "m=video 9 UDP/TLS/RTP/SAVPF 124 97 116\r\n",
        "a=rtpmap:124 H264/90000\r\n",
        "a=rtpmap:97 rtx/90000\r\n",
        "a=fmtp:97 apt=124\r\n",
        "a=rtpmap:116 ulpfec/90000\r\n",
        "a=ssrc-group:FID 1111 99\r\n",
    ));
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
    let mut sink = build_sink(tx);
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
    let mut sink = build_sink(tx);
    sink.payload_route_map = parse_payload_route_map_from_answer(concat!(
        "v=0\r\n",
        "m=video 9 UDP/TLS/RTP/SAVPF 124 97\r\n",
        "a=rtpmap:124 H264/90000\r\n",
        "a=rtpmap:97 rtx/90000\r\n",
        "a=fmtp:97 apt=124\r\n",
        "a=ssrc-group:FID 1111 99\r\n",
    ));
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
    let mut sink = build_sink(tx);
    sink.payload_route_map = parse_payload_route_map_from_answer(concat!(
        "v=0\r\n",
        "m=video 9 UDP/TLS/RTP/SAVPF 124 97\r\n",
        "a=rtpmap:124 H264/90000\r\n",
        "a=rtpmap:97 rtx/90000\r\n",
        "a=fmtp:97 apt=124\r\n",
        "a=ssrc-group:FID 1111 99\r\n",
    ));
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
    let mut sink = build_sink(tx);
    sink.payload_route_map = parse_payload_route_map_from_answer(concat!(
        "v=0\r\n",
        "m=video 9 UDP/TLS/RTP/SAVPF 124 97\r\n",
        "a=rtpmap:124 H264/90000\r\n",
        "a=rtpmap:97 rtx/90000\r\n",
        "a=ssrc-group:FID 1111 99\r\n",
    ));
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
    let mut sink = build_sink(tx);
    sink.payload_route_map = parse_payload_route_map_from_answer(concat!(
        "v=0\r\n",
        "m=video 9 UDP/TLS/RTP/SAVPF 124 97\r\n",
        "a=rtpmap:124 H264/90000\r\n",
        "a=rtpmap:97 rtx/90000\r\n",
        "a=fmtp:97 apt=124\r\n",
    ));
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
    let mut sink = build_sink(tx);
    sink.payload_route_map = parse_payload_route_map_from_answer(concat!(
        "v=0\r\n",
        "m=video 9 UDP/TLS/RTP/SAVPF 124 97\r\n",
        "a=rtpmap:124 H264/90000\r\n",
        "a=rtpmap:97 rtx/90000\r\n",
        "a=fmtp:97 apt=124\r\n",
    ));
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
    let mut sink = build_sink(tx);
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
async fn full_queue_keeps_latest_best_effort_packet_locally() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let mut sink = build_sink(tx);
    let make_packet = |sequence_number: u16| {
        let packet = RtcMediaIngressPacket::new(
            MediaPacketKind::Rtp,
            4,
            RtcMediaPacketSource::Track {
                track_id: "video-main".to_string(),
            },
        )
        .with_rtp_payload(vec![0x41, 0x88, 0x81, 0x00]);
        let meta = RtcRtpPacketMeta {
            ssrc: 7,
            payload_type: 124,
            sequence_number,
            timestamp: 1000 + u32::from(sequence_number),
            marker: false,
        };
        (packet, meta)
    };

    let (packet1, meta1) = make_packet(1);
    sink.on_raw_packet(
        &packet1,
        RtcMediaRouteLabel::PrimaryVideo,
        "route=primaryVideo",
        Some(&meta1),
    );
    let (packet2, meta2) = make_packet(2);
    sink.on_raw_packet(
        &packet2,
        RtcMediaRouteLabel::PrimaryVideo,
        "route=primaryVideo",
        Some(&meta2),
    );

    assert_eq!(sink.pending_priority.len(), 0);
    assert_eq!(
        sink.pending_best_effort
            .as_ref()
            .map(|packet| packet.meta.sequence_number),
        Some(2)
    );

    let first = rx
        .recv()
        .await
        .expect("channel should contain first packet");
    assert_eq!(first.meta.sequence_number, 1);
    sink.flush_pending();
    let second = rx
        .recv()
        .await
        .expect("pending best-effort should be flushed");
    assert_eq!(second.meta.sequence_number, 2);
}

#[tokio::test]
async fn priority_packet_is_buffered_ahead_of_best_effort_under_backpressure() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let mut sink = build_sink(tx);

    let best_effort_packet = RtcMediaIngressPacket::new(
        MediaPacketKind::Rtp,
        4,
        RtcMediaPacketSource::Track {
            track_id: "video-main".to_string(),
        },
    )
    .with_rtp_payload(vec![0x41, 0x88, 0x81, 0x00]);
    let best_effort_meta = RtcRtpPacketMeta {
        ssrc: 7,
        payload_type: 124,
        sequence_number: 10,
        timestamp: 1010,
        marker: false,
    };
    sink.on_raw_packet(
        &best_effort_packet,
        RtcMediaRouteLabel::PrimaryVideo,
        "route=primaryVideo",
        Some(&best_effort_meta),
    );

    let pending_best_effort_packet = RtcMediaIngressPacket::new(
        MediaPacketKind::Rtp,
        4,
        RtcMediaPacketSource::Track {
            track_id: "video-main".to_string(),
        },
    )
    .with_rtp_payload(vec![0x41, 0x88, 0x81, 0x00]);
    let pending_best_effort_meta = RtcRtpPacketMeta {
        ssrc: 7,
        payload_type: 124,
        sequence_number: 11,
        timestamp: 1011,
        marker: false,
    };
    sink.on_raw_packet(
        &pending_best_effort_packet,
        RtcMediaRouteLabel::PrimaryVideo,
        "route=primaryVideo",
        Some(&pending_best_effort_meta),
    );

    let priority_packet = RtcMediaIngressPacket::new(
        MediaPacketKind::Rtp,
        4,
        RtcMediaPacketSource::Track {
            track_id: "video-main".to_string(),
        },
    )
    .with_rtp_payload(vec![0x65, 0x88, 0x81, 0x00]);
    let priority_meta = RtcRtpPacketMeta {
        ssrc: 7,
        payload_type: 124,
        sequence_number: 12,
        timestamp: 1012,
        marker: true,
    };
    sink.on_raw_packet(
        &priority_packet,
        RtcMediaRouteLabel::PrimaryVideo,
        "route=primaryVideo",
        Some(&priority_meta),
    );

    assert_eq!(
        sink.pending_priority
            .front()
            .map(|packet| packet.meta.sequence_number),
        Some(12)
    );
    assert_eq!(
        sink.pending_best_effort
            .as_ref()
            .map(|packet| packet.meta.sequence_number),
        Some(11)
    );

    let first = rx.recv().await.expect("first packet should exist");
    assert_eq!(first.meta.sequence_number, 10);
    sink.flush_pending();
    let second = rx.recv().await.expect("priority packet should flush first");
    assert_eq!(second.meta.sequence_number, 12);
    sink.flush_pending();
    let third = rx.recv().await.expect("best-effort should flush last");
    assert_eq!(third.meta.sequence_number, 11);
}

#[tokio::test]
async fn replacing_pending_best_effort_records_local_backpressure_drop() {
    let (runtime_stats, sink_stats) = runtime_stats_pair();
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    let mut sink = RtcVideoSourceSink::new(tx, sink_stats);

    let make_packet = |sequence_number: u16| {
        let packet = RtcMediaIngressPacket::new(
            MediaPacketKind::Rtp,
            4,
            RtcMediaPacketSource::Track {
                track_id: "video-main".to_string(),
            },
        )
        .with_rtp_payload(vec![0x41, 0x88, 0x81, 0x00]);
        let meta = RtcRtpPacketMeta {
            ssrc: 7,
            payload_type: 124,
            sequence_number,
            timestamp: 2000 + u32::from(sequence_number),
            marker: false,
        };
        (packet, meta)
    };

    let (packet1, meta1) = make_packet(1);
    sink.on_raw_packet(
        &packet1,
        RtcMediaRouteLabel::PrimaryVideo,
        "route=primaryVideo",
        Some(&meta1),
    );
    let (packet2, meta2) = make_packet(2);
    sink.on_raw_packet(
        &packet2,
        RtcMediaRouteLabel::PrimaryVideo,
        "route=primaryVideo",
        Some(&meta2),
    );
    let (packet3, meta3) = make_packet(3);
    sink.on_raw_packet(
        &packet3,
        RtcMediaRouteLabel::PrimaryVideo,
        "route=primaryVideo",
        Some(&meta3),
    );

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let drop = stats
        .latest_video_frame_drop
        .as_ref()
        .expect("local backpressure drop should be recorded");
    assert_eq!(drop.reason, "localBackpressureBestEffortReplaced");
    assert_eq!(drop.stage.as_deref(), Some("ingress"));
    assert_eq!(drop.action.as_deref(), Some("drop"));
    assert_eq!(
        drop.detail.as_deref(),
        Some("bestEffortReplacedByNewerPacket:best-effort")
    );
    assert_eq!(drop.frame_rtp_timestamp, Some(2002));
    assert_eq!(
        drop.frame_unrecoverable_reason.as_deref(),
        Some("localBackpressure")
    );
}
