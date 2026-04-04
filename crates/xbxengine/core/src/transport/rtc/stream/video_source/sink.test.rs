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
