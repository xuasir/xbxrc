use crate::media::video::test_fixtures::bootstrap_pps_nalu;
use crate::transport::rtc::facts::{
    ConnectionLifecycleStateFact, SessionCommand, TransportCommand,
};
use crate::transport::rtc::projection::{
    BweProjection, ConnectionProjection, DiagnosticsProjection, MediaProjection,
    RecoveryProjection, TransportSnapshot,
};
use crate::transport::rtc::receive::ingress_state::FrameBoundaryTracker;
use crate::transport::rtc::receive::rtx_sink::RtcVideoSourceSink;
use crate::transport::rtc::receive::test_fixtures::{
    run_local_ingress_replay_profile, runtime_stats_pair, LocalIngressHealthyBaseline,
    LocalIngressReplayFixture, LocalIngressReplayPacket, LocalIngressReplayProfile,
};
use crate::transport::rtc::session::actor::SessionPolicyHook;
use crate::transport::rtc::session::policy::RtcSessionPolicy;
use crate::transport::rtc::stream::packet_router::parse_payload_route_map_from_answer;
use crate::transport::rtc::stream::packet_router::RtcMediaRouteLabel;
use crate::transport::rtc::stream::packet_types::{
    MediaPacketKind, RtcMediaIngressPacket, RtcMediaPacketSource, RtcRtpPacketMeta,
    RtcVideoIngressKind,
};
use crate::transport::rtc::stream::sink::RtcMediaSink;
use crate::XbxEngineRecoveryReasonDomain;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn assert_no_reconnect_candidate(commands: &[TransportCommand]) {
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
}

fn assert_has_connectivity_reconnect_candidate(commands: &[TransportCommand], reason: &str) {
    assert!(commands.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestReconnectCandidate {
                reason: command_reason,
                reason_domain,
                ..
            } if command_reason == reason
                && *reason_domain == XbxEngineRecoveryReasonDomain::ConnectivityTransport
        )
    }));
}

fn transport_commands(commands: Vec<SessionCommand>) -> Vec<TransportCommand> {
    commands
        .into_iter()
        .filter_map(|command| match command {
            SessionCommand::Transport(command) => Some(command),
            SessionCommand::LocalDecoderReset { .. } => None,
        })
        .collect()
}

fn assert_latest_recovery_input_signal(
    runtime_stats: &Arc<Mutex<crate::XbxEngineMediaRuntimeStats>>,
    expected_input_signal: &str,
) {
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(ledger.input_signal, expected_input_signal);
}

fn build_sink(
    tx: tokio::sync::mpsc::Sender<crate::transport::rtc::stream::packet_types::RtcVideoRtpPacket>,
) -> RtcVideoSourceSink {
    let (_, runtime_stats) = runtime_stats_pair();
    RtcVideoSourceSink::new(
        tx,
        runtime_stats,
        Arc::new(Mutex::new(FrameBoundaryTracker::new())),
    )
}

fn repair_overflow_replay_profile(repair_limit: usize) -> LocalIngressReplayProfile {
    let _ = repair_limit;
    LocalIngressReplayProfile {
        channel_capacity: 1,
        // 与 repair_overflow_drops_oldest_* 一致：需要超过 repair backlog 的注入量才会记录 drop。
        packets: (10u16..=16)
            .map(|seq| LocalIngressReplayPacket {
                payload_type: 124,
                sequence_number: seq,
                timestamp: 4_000 + u32::from(seq),
                payload: vec![0x41, 0x88, 0x81, 0x00],
            })
            .collect(),
        baseline: LocalIngressHealthyBaseline {
            now_ms: 9_000.0,
            frame_rtp_timestamp: 4_016,
        },
    }
}

fn unmatched_repair_rtx_burst_replay_profile() -> LocalIngressReplayProfile {
    LocalIngressReplayProfile {
        channel_capacity: 4,
        packets: (0..5u16)
            .map(|offset| {
                let mut payload = Vec::with_capacity(2 + bootstrap_pps_nalu().len());
                payload.extend_from_slice(&(8_000 + offset).to_be_bytes());
                payload.extend_from_slice(&bootstrap_pps_nalu());
                LocalIngressReplayPacket {
                    payload_type: 97,
                    sequence_number: 300 + offset,
                    timestamp: 12_000,
                    payload,
                }
            })
            .collect(),
        baseline: LocalIngressHealthyBaseline {
            now_ms: 12_500.0,
            frame_rtp_timestamp: 12_000,
        },
    }
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
async fn repair_rtx_packet_with_non_primary_apt_target_is_ignored() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let mut sink = build_sink(tx);
    sink.payload_route_map = parse_payload_route_map_from_answer(concat!(
        "v=0\r\n",
        "m=video 9 UDP/TLS/RTP/SAVPF 124 97 125\r\n",
        "a=rtpmap:124 H264/90000\r\n",
        "a=rtpmap:97 rtx/90000\r\n",
        "a=fmtp:97 apt=125\r\n",
        "a=rtpmap:125 red/90000\r\n",
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

    assert_eq!(sink.test_pending_priority_primary_len(), 0);
    assert_eq!(sink.test_pending_best_effort_front_sequence(), Some(2));

    let first = rx
        .recv()
        .await
        .expect("channel should contain first packet");
    assert_eq!(first.meta.sequence_number, 1);
    sink.test_flush_pending();
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
        sink.test_pending_priority_primary_front_sequence(),
        Some(12)
    );
    assert_eq!(sink.test_pending_best_effort_front_sequence(), Some(11));

    let first = rx.recv().await.expect("first packet should exist");
    assert_eq!(first.meta.sequence_number, 10);
    sink.test_flush_pending();
    let second = rx.recv().await.expect("priority packet should flush first");
    assert_eq!(second.meta.sequence_number, 12);
    sink.test_flush_pending();
    let third = rx.recv().await.expect("best-effort should flush last");
    assert_eq!(third.meta.sequence_number, 11);
}

#[tokio::test]
async fn replacing_pending_best_effort_records_local_backpressure_drop() {
    let (runtime_stats, sink_stats) = runtime_stats_pair();
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    let mut sink = RtcVideoSourceSink::new(
        tx,
        sink_stats,
        Arc::new(Mutex::new(FrameBoundaryTracker::new())),
    );

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
    assert_eq!(drop.reason, "localBackpressureBestEffortOverflow");
    assert_eq!(drop.stage.as_deref(), Some("ingress"));
    assert_eq!(drop.action.as_deref(), Some("drop"));
    assert_eq!(
        drop.detail.as_deref(),
        Some("bestEffortQueueDropOldest:best-effort")
    );
    assert_eq!(drop.frame_rtp_timestamp, Some(2002));
    assert_eq!(
        drop.frame_unrecoverable_reason.as_deref(),
        Some("localBackpressure")
    );
    let breakdown = drop
        .ingress_queue_depth_breakdown
        .as_ref()
        .expect("ingress queue depth breakdown should be recorded");
    assert_eq!(breakdown.sender_max_capacity, 1);
    assert_eq!(breakdown.sender_queue_limit, 1);
    assert_eq!(breakdown.sender_queue_depth, 1);
    assert_eq!(breakdown.sender_remaining_capacity, 0);
    assert_eq!(breakdown.pending_priority_primary_limit, 4);
    assert_eq!(breakdown.pending_repair_limit, 4);
    assert_eq!(breakdown.pending_best_effort_limit, 1);
    assert_eq!(drop.queue_depth, breakdown.total_queue_depth());
}

#[tokio::test]
async fn oversized_sender_channel_is_clamped_to_low_latency_depth() {
    let (runtime_stats, sink_stats) = runtime_stats_pair();
    let (tx, _rx) = tokio::sync::mpsc::channel(128);
    let mut sink = RtcVideoSourceSink::new(
        tx,
        sink_stats,
        Arc::new(Mutex::new(FrameBoundaryTracker::new())),
    );

    for sequence_number in 1u16..=67 {
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
            timestamp: 3_000 + u32::from(sequence_number),
            marker: false,
        };
        sink.on_raw_packet(
            &packet,
            RtcMediaRouteLabel::PrimaryVideo,
            "route=primaryVideo",
            Some(&meta),
        );
    }

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let drop = stats
        .latest_video_frame_drop
        .as_ref()
        .expect("oversized sender should still enter local backpressure");
    assert_eq!(drop.reason, "localBackpressureBestEffortOverflow");
    let breakdown = drop
        .ingress_queue_depth_breakdown
        .as_ref()
        .expect("ingress queue depth breakdown should be recorded");
    assert_eq!(breakdown.sender_max_capacity, 128);
    assert_eq!(breakdown.sender_queue_limit, 64);
    assert_eq!(breakdown.sender_queue_depth, 48);
    assert_eq!(breakdown.pending_best_effort_limit, 2);
    assert_eq!(drop.queue_depth, breakdown.total_queue_depth());
    assert!(drop.queue_depth < 128);
}

#[tokio::test]
async fn best_effort_uses_soft_watermark_before_sender_hard_limit() {
    let (runtime_stats, sink_stats) = runtime_stats_pair();
    let (tx, _rx) = tokio::sync::mpsc::channel(128);
    let mut sink = RtcVideoSourceSink::new(
        tx,
        sink_stats,
        Arc::new(Mutex::new(FrameBoundaryTracker::new())),
    );

    for sequence_number in 1u16..=51 {
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
            timestamp: 3_500 + u32::from(sequence_number),
            marker: false,
        };
        sink.on_raw_packet(
            &packet,
            RtcMediaRouteLabel::PrimaryVideo,
            "route=primaryVideo",
            Some(&meta),
        );
    }

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let drop = stats
        .latest_video_frame_drop
        .as_ref()
        .expect("best-effort soft watermark should record overflow");
    assert_eq!(drop.reason, "localBackpressureBestEffortOverflow");
    let breakdown = drop
        .ingress_queue_depth_breakdown
        .as_ref()
        .expect("ingress queue depth breakdown should be recorded");
    assert_eq!(breakdown.sender_max_capacity, 128);
    assert_eq!(breakdown.sender_queue_limit, 64);
    assert_eq!(breakdown.sender_queue_depth, 48);
    assert_eq!(breakdown.pending_best_effort_limit, 2);
    assert_eq!(breakdown.pending_best_effort_len, 1);
    assert!(drop.queue_depth < breakdown.sender_queue_limit);
}

#[tokio::test]
async fn keyframe_fu_continuation_inherits_priority_before_ingress_loop_consumes_head() {
    let (runtime_stats, sink_stats) = runtime_stats_pair();
    let (tx, mut rx) = tokio::sync::mpsc::channel(128);
    let mut sink = RtcVideoSourceSink::new(
        tx,
        sink_stats,
        Arc::new(Mutex::new(FrameBoundaryTracker::new())),
    );

    for sequence_number in 1u16..=48 {
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
            timestamp: 9_000 + u32::from(sequence_number),
            marker: false,
        };
        sink.on_raw_packet(
            &packet,
            RtcMediaRouteLabel::PrimaryVideo,
            "route=primaryVideo",
            Some(&meta),
        );
    }

    let idr_start = RtcMediaIngressPacket::new(
        MediaPacketKind::Rtp,
        4,
        RtcMediaPacketSource::Track {
            track_id: "video-main".to_string(),
        },
    )
    .with_rtp_payload(vec![0x7c, 0x85, 0x11, 0x22]);
    let idr_start_meta = RtcRtpPacketMeta {
        ssrc: 7,
        payload_type: 124,
        sequence_number: 100,
        timestamp: 42_000,
        marker: false,
    };
    sink.on_raw_packet(
        &idr_start,
        RtcMediaRouteLabel::PrimaryVideo,
        "route=primaryVideo",
        Some(&idr_start_meta),
    );

    let idr_continuation = RtcMediaIngressPacket::new(
        MediaPacketKind::Rtp,
        4,
        RtcMediaPacketSource::Track {
            track_id: "video-main".to_string(),
        },
    )
    .with_rtp_payload(vec![0x7c, 0x05, 0x33, 0x44]);
    let idr_continuation_meta = RtcRtpPacketMeta {
        ssrc: 7,
        payload_type: 124,
        sequence_number: 101,
        timestamp: 42_000,
        marker: true,
    };
    sink.on_raw_packet(
        &idr_continuation,
        RtcMediaRouteLabel::PrimaryVideo,
        "route=primaryVideo",
        Some(&idr_continuation_meta),
    );

    assert_eq!(sink.test_pending_best_effort_front_sequence(), None);
    assert_eq!(sink.test_pending_priority_primary_len(), 0);
    let mut received_sequences = Vec::new();
    while let Ok(packet) = rx.try_recv() {
        received_sequences.push(packet.meta.sequence_number);
    }
    assert!(received_sequences.contains(&100));
    assert!(received_sequences.contains(&101));
    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert!(stats.latest_video_frame_drop.is_none());
}

#[tokio::test]
async fn keyframe_fu_continuation_inherits_priority_when_sender_queue_is_full() {
    let (_runtime_stats, sink_stats) = runtime_stats_pair();
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    let mut sink = RtcVideoSourceSink::new(
        tx,
        sink_stats,
        Arc::new(Mutex::new(FrameBoundaryTracker::new())),
    );

    let fill_packet = RtcMediaIngressPacket::new(
        MediaPacketKind::Rtp,
        4,
        RtcMediaPacketSource::Track {
            track_id: "video-main".to_string(),
        },
    )
    .with_rtp_payload(vec![0x41, 0x88, 0x81, 0x00]);
    let fill_meta = RtcRtpPacketMeta {
        ssrc: 7,
        payload_type: 124,
        sequence_number: 1,
        timestamp: 12_000,
        marker: false,
    };
    sink.on_raw_packet(
        &fill_packet,
        RtcMediaRouteLabel::PrimaryVideo,
        "route=primaryVideo",
        Some(&fill_meta),
    );

    let idr_start = RtcMediaIngressPacket::new(
        MediaPacketKind::Rtp,
        4,
        RtcMediaPacketSource::Track {
            track_id: "video-main".to_string(),
        },
    )
    .with_rtp_payload(vec![0x7c, 0x85, 0x11, 0x22]);
    let idr_start_meta = RtcRtpPacketMeta {
        ssrc: 7,
        payload_type: 124,
        sequence_number: 100,
        timestamp: 43_000,
        marker: false,
    };
    sink.on_raw_packet(
        &idr_start,
        RtcMediaRouteLabel::PrimaryVideo,
        "route=primaryVideo",
        Some(&idr_start_meta),
    );

    let idr_continuation = RtcMediaIngressPacket::new(
        MediaPacketKind::Rtp,
        4,
        RtcMediaPacketSource::Track {
            track_id: "video-main".to_string(),
        },
    )
    .with_rtp_payload(vec![0x7c, 0x05, 0x33, 0x44]);
    let idr_continuation_meta = RtcRtpPacketMeta {
        ssrc: 7,
        payload_type: 124,
        sequence_number: 101,
        timestamp: 43_000,
        marker: true,
    };
    sink.on_raw_packet(
        &idr_continuation,
        RtcMediaRouteLabel::PrimaryVideo,
        "route=primaryVideo",
        Some(&idr_continuation_meta),
    );

    assert_eq!(sink.test_pending_priority_primary_len(), 2);
    assert_eq!(
        sink.test_pending_priority_primary_front_sequence(),
        Some(100)
    );
    assert_eq!(sink.test_pending_best_effort_front_sequence(), None);
}

#[tokio::test]
async fn dropped_keyframe_head_does_not_pollute_priority_for_later_continuation() {
    let (_runtime_stats, sink_stats) = runtime_stats_pair();
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    let mut sink = RtcVideoSourceSink::new(
        tx,
        sink_stats,
        Arc::new(Mutex::new(FrameBoundaryTracker::new())),
    );

    let fill_packet = RtcMediaIngressPacket::new(
        MediaPacketKind::Rtp,
        4,
        RtcMediaPacketSource::Track {
            track_id: "video-main".to_string(),
        },
    )
    .with_rtp_payload(vec![0x41, 0x88, 0x81, 0x00]);
    let fill_meta = RtcRtpPacketMeta {
        ssrc: 7,
        payload_type: 124,
        sequence_number: 1,
        timestamp: 20_000,
        marker: false,
    };
    sink.on_raw_packet(
        &fill_packet,
        RtcMediaRouteLabel::PrimaryVideo,
        "route=primaryVideo",
        Some(&fill_meta),
    );

    for offset in 0u16..4 {
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
            sequence_number: 100 + offset,
            timestamp: 44_000 + u32::from(offset),
            marker: true,
        };
        sink.on_raw_packet(
            &packet,
            RtcMediaRouteLabel::PrimaryVideo,
            "route=primaryVideo",
            Some(&meta),
        );
    }

    let dropped_idr_head = RtcMediaIngressPacket::new(
        MediaPacketKind::Rtp,
        4,
        RtcMediaPacketSource::Track {
            track_id: "video-main".to_string(),
        },
    )
    .with_rtp_payload(vec![0x7c, 0x85, 0x11, 0x22]);
    let dropped_idr_head_meta = RtcRtpPacketMeta {
        ssrc: 7,
        payload_type: 124,
        sequence_number: 200,
        timestamp: 45_000,
        marker: false,
    };
    sink.on_raw_packet(
        &dropped_idr_head,
        RtcMediaRouteLabel::PrimaryVideo,
        "route=primaryVideo",
        Some(&dropped_idr_head_meta),
    );

    let continuation = RtcMediaIngressPacket::new(
        MediaPacketKind::Rtp,
        4,
        RtcMediaPacketSource::Track {
            track_id: "video-main".to_string(),
        },
    )
    .with_rtp_payload(vec![0x7c, 0x05, 0x33, 0x44]);
    let continuation_meta = RtcRtpPacketMeta {
        ssrc: 7,
        payload_type: 124,
        sequence_number: 201,
        timestamp: 45_000,
        marker: true,
    };
    sink.on_raw_packet(
        &continuation,
        RtcMediaRouteLabel::PrimaryVideo,
        "route=primaryVideo",
        Some(&continuation_meta),
    );

    assert_eq!(sink.test_pending_priority_primary_len(), 4);
    assert_eq!(sink.test_pending_best_effort_front_sequence(), Some(201));
}

#[tokio::test]
async fn repair_noise_downgrades_to_best_effort_under_sender_pressure() {
    let (runtime_stats, sink_stats) = runtime_stats_pair();
    let (tx, _rx) = tokio::sync::mpsc::channel(128);
    let mut sink = RtcVideoSourceSink::new(
        tx,
        sink_stats,
        Arc::new(Mutex::new(FrameBoundaryTracker::new())),
    );
    sink.payload_route_map = parse_payload_route_map_from_answer(concat!(
        "v=0\r\n",
        "m=video 9 UDP/TLS/RTP/SAVPF 124 97\r\n",
        "a=rtpmap:124 H264/90000\r\n",
        "a=rtpmap:97 rtx/90000\r\n",
        "a=fmtp:97 apt=124\r\n",
        "a=ssrc-group:FID 1111 99\r\n",
    ));

    for sequence_number in 1u16..=48 {
        let packet = RtcMediaIngressPacket::new(
            MediaPacketKind::Rtp,
            4,
            RtcMediaPacketSource::Track {
                track_id: "video-main".to_string(),
            },
        )
        .with_rtp_payload(vec![0x41, 0x88, 0x81, 0x00]);
        let meta = RtcRtpPacketMeta {
            ssrc: 1111,
            payload_type: 124,
            sequence_number,
            timestamp: 4_500 + u32::from(sequence_number),
            marker: false,
        };
        sink.on_raw_packet(
            &packet,
            RtcMediaRouteLabel::PrimaryVideo,
            "route=primaryVideo",
            Some(&meta),
        );
    }

    for sequence_number in 49u16..=51 {
        let packet = RtcMediaIngressPacket::new(
            MediaPacketKind::Rtp,
            4,
            RtcMediaPacketSource::Track {
                track_id: "video-repair".to_string(),
            },
        )
        .with_rtp_payload(vec![0x41, 0x88, 0x81, 0x00]);
        let meta = RtcRtpPacketMeta {
            ssrc: 99,
            payload_type: 124,
            sequence_number,
            timestamp: 4_500 + u32::from(sequence_number),
            marker: false,
        };
        sink.on_raw_packet(
            &packet,
            RtcMediaRouteLabel::RepairVideo,
            "route=repairVideo",
            Some(&meta),
        );
    }

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let drop = stats
        .latest_video_frame_drop
        .as_ref()
        .expect("repair pressure should downgrade into best-effort overflow");
    assert_eq!(drop.reason, "localBackpressureBestEffortOverflow");
    assert_eq!(
        drop.detail.as_deref(),
        Some("bestEffortQueueDropOldest:priority-repair")
    );
    let breakdown = drop
        .ingress_queue_depth_breakdown
        .as_ref()
        .expect("ingress queue depth breakdown should be recorded");
    assert_eq!(breakdown.pending_repair_len, 0);
    assert_eq!(breakdown.pending_best_effort_len, 1);
    assert_eq!(breakdown.sender_queue_depth, 48);
}

#[tokio::test]
async fn enqueue_under_backpressure_schedules_follow_up_flush() {
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
            timestamp: 4_000 + u32::from(sequence_number),
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

    assert!(sink.test_has_scheduled_flush());
    let first = rx.recv().await.expect("first packet should be queued");
    assert_eq!(first.meta.sequence_number, 1);
}

#[tokio::test]
async fn due_scheduled_flush_runs_on_tick() {
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
            timestamp: 5_000 + u32::from(sequence_number),
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
    assert!(sink.test_has_scheduled_flush());

    let first = rx.recv().await.expect("first packet should exist");
    assert_eq!(first.meta.sequence_number, 1);

    // 模拟 tick task 到期触发：直接调用 on_tick 并传入已到期的时间点
    sink.test_force_flush_due_now();
    sink.on_tick(Instant::now());

    let second = tokio::time::timeout(Duration::from_millis(50), rx.recv())
        .await
        .expect("on_tick should flush pending packet")
        .expect("pending packet should exist");
    assert_eq!(second.meta.sequence_number, 2);
}

#[tokio::test]
async fn draining_pending_queue_clears_scheduled_flush() {
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
            timestamp: 6_000 + u32::from(sequence_number),
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
    assert!(sink.test_has_scheduled_flush());

    let first = rx.recv().await.expect("first packet should exist");
    assert_eq!(first.meta.sequence_number, 1);

    sink.test_flush_pending();
    let second = tokio::time::timeout(Duration::from_millis(50), rx.recv())
        .await
        .expect("flush should enqueue pending packet")
        .expect("pending packet should exist");
    assert_eq!(second.meta.sequence_number, 2);
    assert!(!sink.test_has_scheduled_flush());
}

#[tokio::test]
async fn repair_queue_overflow_drops_oldest_pending_repair_packet() {
    let (runtime_stats, sink_stats) = runtime_stats_pair();
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    let mut sink = RtcVideoSourceSink::new(
        tx,
        sink_stats,
        Arc::new(Mutex::new(FrameBoundaryTracker::new())),
    );
    sink.payload_route_map = parse_payload_route_map_from_answer(concat!(
        "v=0\r\n",
        "m=video 9 UDP/TLS/RTP/SAVPF 124 97\r\n",
        "a=rtpmap:124 H264/90000\r\n",
        "a=rtpmap:97 rtx/90000\r\n",
        "a=fmtp:97 apt=124\r\n",
        "a=ssrc-group:FID 1111 99\r\n",
    ));

    let first_packet = RtcMediaIngressPacket::new(
        MediaPacketKind::Rtp,
        4,
        RtcMediaPacketSource::Track {
            track_id: "video-main".to_string(),
        },
    )
    .with_rtp_payload(vec![0x41, 0x88, 0x81, 0x00]);
    let first_meta = RtcRtpPacketMeta {
        ssrc: 7,
        payload_type: 124,
        sequence_number: 1,
        timestamp: 1001,
        marker: false,
    };
    sink.on_raw_packet(
        &first_packet,
        RtcMediaRouteLabel::PrimaryVideo,
        "route=primaryVideo",
        Some(&first_meta),
    );

    let repair_packet = RtcMediaIngressPacket::new(
        MediaPacketKind::Rtp,
        4,
        RtcMediaPacketSource::Track {
            track_id: "video-repair".to_string(),
        },
    )
    .with_rtp_payload(vec![0x41, 0x88, 0x81, 0x00]);
    let repair_limit = sink.test_repair_backlog_limit();
    for offset in 0..=repair_limit {
        let seq = 10 + offset as u16;
        let meta = RtcRtpPacketMeta {
            ssrc: 99,
            payload_type: 124,
            sequence_number: seq,
            timestamp: 2000 + u32::from(seq),
            marker: false,
        };
        sink.on_raw_packet(
            &repair_packet,
            RtcMediaRouteLabel::RepairVideo,
            "route=repairVideo",
            Some(&meta),
        );
    }

    let pending = sink.test_pending_repair_sequences();
    let expected = (1..=repair_limit)
        .map(|offset| 10 + offset as u16)
        .collect::<Vec<_>>();
    assert_eq!(pending.len(), repair_limit);
    assert_eq!(pending, expected);

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let drop = stats
        .latest_video_frame_drop
        .as_ref()
        .expect("repair overflow drop should be recorded");
    assert_eq!(drop.reason, "localBackpressureRepairOverflow");
    assert_eq!(
        drop.detail.as_deref(),
        Some("repairQueueDropOldest:priority-repair")
    );
    assert_eq!(drop.frame_rtp_timestamp, Some(2010));
    assert_eq!(
        drop.frame_unrecoverable_reason.as_deref(),
        Some("localBackpressure")
    );
}

#[tokio::test]
async fn repair_overflow_drops_oldest_repair_and_records_backpressure() {
    let (runtime_stats, sink_stats) = runtime_stats_pair();
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    let mut sink = RtcVideoSourceSink::new(
        tx,
        sink_stats,
        Arc::new(Mutex::new(FrameBoundaryTracker::new())),
    );
    sink.payload_route_map = parse_payload_route_map_from_answer(concat!(
        "v=0\r\n",
        "m=video 9 UDP/TLS/RTP/SAVPF 124 97\r\n",
        "a=rtpmap:124 H264/90000\r\n",
        "a=rtpmap:97 rtx/90000\r\n",
        "a=fmtp:97 apt=124\r\n",
        "a=ssrc-group:FID 1111 99\r\n",
    ));

    let repair_packet = RtcMediaIngressPacket::new(
        MediaPacketKind::Rtp,
        4,
        RtcMediaPacketSource::Track {
            track_id: "video-repair".to_string(),
        },
    )
    .with_rtp_payload(vec![0x41, 0x88, 0x81, 0x00]);

    for seq in 10u16..=16 {
        let repair_meta = RtcRtpPacketMeta {
            ssrc: 99,
            payload_type: 124,
            sequence_number: seq,
            timestamp: 3000 + u32::from(seq),
            marker: false,
        };
        sink.on_raw_packet(
            &repair_packet,
            RtcMediaRouteLabel::RepairVideo,
            "route=repairVideo",
            Some(&repair_meta),
        );
    }

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let drop = stats
        .latest_video_frame_drop
        .as_ref()
        .expect("repair overflow drop should be recorded");
    assert_eq!(drop.reason, "localBackpressureRepairOverflow");
    assert_eq!(drop.stage.as_deref(), Some("ingress"));
    assert_eq!(drop.action.as_deref(), Some("drop"));
    assert_eq!(
        drop.detail.as_deref(),
        Some("repairQueueDropOldest:priority-repair")
    );
    assert_eq!(drop.frame_rtp_timestamp, Some(3012));
    assert_eq!(
        drop.frame_unrecoverable_reason.as_deref(),
        Some("localBackpressure")
    );
}

#[tokio::test]
#[ignore = "过时 replay harness：Phase C 收口前不跑 drain+policy 集成"]
async fn repair_overflow_drains_into_source_but_stays_local_without_transport_observation() {
    let repair_limit = LocalIngressReplayFixture::new(1).repair_backlog_limit();
    let profile = repair_overflow_replay_profile(repair_limit);
    let harness = run_local_ingress_replay_profile(&profile).await;

    {
        let runtime_stats = harness.runtime_stats();
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let drop = stats
            .latest_video_frame_drop
            .as_ref()
            .expect("repair overflow drop should be recorded");
        assert_eq!(drop.reason, "localBackpressureRepairOverflow");
        assert_eq!(
            drop.detail.as_deref(),
            Some("repairQueueDropOldest:priority-repair")
        );
    }

    let commands = harness.run_policy_snapshot(profile.baseline.now_ms);
    assert_no_reconnect_candidate(&commands);
}

#[tokio::test]
#[ignore = "过时 replay harness：Phase C 收口前不跑 drain+policy 集成"]
async fn unmatched_repair_rtx_burst_through_real_ingress_stays_local_without_transport_observation()
{
    let profile = unmatched_repair_rtx_burst_replay_profile();
    let harness = run_local_ingress_replay_profile(&profile).await;

    {
        let runtime_stats = harness.runtime_stats();
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let latest = stats
            .latest_video_rtx_reinject_observation
            .as_ref()
            .expect("repair burst observation should be recorded");
        assert_eq!(latest.stage, "adapterResolveMiss");
        assert_eq!(latest.sequence_number, 8_004);
        assert_eq!(latest.repair_ssrc, 99);
        assert_eq!(latest.primary_ssrc, 1111);
        assert_eq!(latest.native_sequence_number, Some(304));
        assert!(!latest.matched_nack_range);
        assert!(!latest.matched_pending_gap);
        assert!(latest.matched_gap_sequence.is_none());
    }

    let commands = harness.run_policy_snapshot(profile.baseline.now_ms);
    assert_no_reconnect_candidate(&commands);
}

#[tokio::test]
#[ignore = "过时 replay harness：Phase C 收口前不跑 drain+policy 集成"]
async fn local_repair_noise_keeps_repeated_transport_severe_deadline_in_local_recovery() {
    let repair_limit = LocalIngressReplayFixture::new(1).repair_backlog_limit();
    let profile = repair_overflow_replay_profile(repair_limit);
    let harness = run_local_ingress_replay_profile(&profile).await;
    let runtime_stats = harness.runtime_stats();
    let mut policy = RtcSessionPolicy::new(
        Arc::new(Mutex::new(
            crate::api::runtime::XbxEngineRuntimeConfig::default(),
        )),
        runtime_stats.clone(),
    );

    let mut healthy_connection = ConnectionProjection::default();
    healthy_connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    healthy_connection.control_channel_open = true;
    healthy_connection.latest_transport_path = Some("Direct".to_string());
    healthy_connection.latest_rtt_ms = Some(42.0);
    healthy_connection.last_observed_at_ms = Some(profile.baseline.now_ms);
    let healthy_snapshot = TransportSnapshot::new(
        1,
        profile.baseline.now_ms,
        healthy_connection,
        MediaProjection {
            frame_count: 240,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(profile.baseline.now_ms),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let healthy_commands = transport_commands(policy.on_snapshot(&healthy_snapshot));
    assert!(healthy_commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));

    harness.mark_transport_connectivity_degraded(profile.baseline.now_ms + 20.0);

    let mut bad_connection = ConnectionProjection::default();
    bad_connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    bad_connection.control_channel_open = true;
    bad_connection.latest_transport_path = Some("Direct".to_string());
    bad_connection.latest_rtt_ms = Some(42.0);
    bad_connection.last_observed_at_ms = Some(profile.baseline.now_ms + 20.0);
    let first_deadline = TransportSnapshot::new(
        2,
        profile.baseline.now_ms + 20.0,
        bad_connection.clone(),
        MediaProjection {
            frame_count: 240,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("transportSevereDeadline".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(profile.baseline.now_ms + 20.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let first_commands = transport_commands(policy.on_snapshot(&first_deadline));
    assert_no_reconnect_candidate(&first_commands);

    let second_deadline = TransportSnapshot::new(
        3,
        profile.baseline.now_ms + 60.0,
        bad_connection,
        MediaProjection {
            frame_count: 240,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("transportSevereDeadline".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(profile.baseline.now_ms + 60.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let second_commands = transport_commands(policy.on_snapshot(&second_deadline));
    assert_no_reconnect_candidate(&second_commands);
}

#[tokio::test]
#[ignore = "过时 replay harness：Phase C 收口前不跑 drain+policy 集成"]
async fn multi_stage_replay_steady_local_noise_severe_then_recover_stays_stable() {
    let repair_limit = LocalIngressReplayFixture::new(1).repair_backlog_limit();
    let profile = repair_overflow_replay_profile(repair_limit);
    let harness = run_local_ingress_replay_profile(&profile).await;
    let runtime_stats = harness.runtime_stats();
    let mut policy = RtcSessionPolicy::new(
        Arc::new(Mutex::new(
            crate::api::runtime::XbxEngineRuntimeConfig::default(),
        )),
        runtime_stats.clone(),
    );

    let steady_snapshot = harness.build_connected_snapshot(1, profile.baseline.now_ms, 240, "none");
    let steady_commands = transport_commands(policy.on_snapshot(&steady_snapshot));
    assert!(
        steady_commands.is_empty(),
        "unexpected steady commands: {steady_commands:?}"
    );

    {
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(
            stats
                .latest_video_frame_drop
                .as_ref()
                .map(|drop| drop.reason.as_str()),
            Some("localBackpressureRepairOverflow")
        );
    }
    let local_noise_snapshot =
        harness.build_connected_snapshot(2, profile.baseline.now_ms + 10.0, 241, "none");
    let local_noise_commands = transport_commands(policy.on_snapshot(&local_noise_snapshot));
    assert!(
        local_noise_commands.is_empty(),
        "unexpected local noise commands: {local_noise_commands:?}"
    );

    harness.mark_transport_connectivity_degraded(profile.baseline.now_ms + 30.0);

    let severe_first = harness.build_connected_snapshot(
        3,
        profile.baseline.now_ms + 30.0,
        241,
        "transportSevereDeadline",
    );
    let severe_first_commands = transport_commands(policy.on_snapshot(&severe_first));
    assert_no_reconnect_candidate(&severe_first_commands);

    harness.inject_transport_await_hard_recovery_bootstrap(profile.baseline.now_ms + 60.0);
    let severe_second_now = profile.baseline.now_ms + 60.0;
    let severe_second = harness.build_broken_connectivity_snapshot(
        4,
        severe_second_now,
        severe_second_now - 3_000.0,
        241,
        "transportSevereDeadline",
    );
    let severe_second_commands = transport_commands(policy.on_snapshot(&severe_second));
    assert_has_connectivity_reconnect_candidate(&severe_second_commands, "transportSevereDeadline");
    assert_latest_recovery_input_signal(
        &runtime_stats,
        "transportSevereDeadline:transportSevereDeadline",
    );

    harness.mark_transport_recovered(profile.baseline.now_ms + 128.0);

    let recovered_snapshot =
        harness.build_connected_snapshot(5, profile.baseline.now_ms + 130.0, 260, "none");
    let recovered_commands = transport_commands(policy.on_snapshot(&recovered_snapshot));
    assert!(
        recovered_commands.is_empty(),
        "unexpected recovered commands: {recovered_commands:?}"
    );
}

#[tokio::test]
#[ignore = "过时 replay harness：Phase C 收口前不跑 drain+policy 集成"]
async fn multi_stage_replay_steady_local_noise_expired_then_recover_stays_stable() {
    let repair_limit = LocalIngressReplayFixture::new(1).repair_backlog_limit();
    let profile = repair_overflow_replay_profile(repair_limit);
    let harness = run_local_ingress_replay_profile(&profile).await;
    let runtime_stats = harness.runtime_stats();
    let mut policy = RtcSessionPolicy::new(
        Arc::new(Mutex::new(
            crate::api::runtime::XbxEngineRuntimeConfig::default(),
        )),
        runtime_stats.clone(),
    );

    let steady_snapshot = harness.build_connected_snapshot(1, profile.baseline.now_ms, 240, "none");
    assert!(
        transport_commands(policy.on_snapshot(&steady_snapshot)).is_empty(),
        "unexpected steady commands"
    );

    let local_noise_snapshot =
        harness.build_connected_snapshot(2, profile.baseline.now_ms + 10.0, 241, "none");
    assert!(
        transport_commands(policy.on_snapshot(&local_noise_snapshot)).is_empty(),
        "unexpected local noise commands"
    );

    harness.mark_transport_connectivity_degraded(profile.baseline.now_ms + 30.0);

    let expired_first = harness.build_connected_snapshot(
        3,
        profile.baseline.now_ms + 30.0,
        241,
        "transportExpiredDeadline",
    );
    let expired_first_commands = transport_commands(policy.on_snapshot(&expired_first));
    assert_no_reconnect_candidate(&expired_first_commands);

    tokio::time::sleep(Duration::from_millis(450)).await;
    let expired_second_now = profile.baseline.now_ms + 450.0;
    harness.inject_transport_await_hard_recovery_bootstrap(expired_second_now);
    let expired_second = harness.build_broken_connectivity_snapshot(
        4,
        expired_second_now,
        expired_second_now - 3_000.0,
        241,
        "transportExpiredDeadline",
    );
    let expired_second_commands = transport_commands(policy.on_snapshot(&expired_second));

    tokio::time::sleep(Duration::from_millis(450)).await;
    let expired_third_now = profile.baseline.now_ms + 900.0;
    harness.inject_transport_await_hard_recovery_bootstrap(expired_third_now);
    let expired_third = harness.build_broken_connectivity_snapshot(
        5,
        expired_third_now,
        expired_third_now - 3_000.0,
        241,
        "transportExpiredDeadline",
    );
    let expired_third_commands = transport_commands(policy.on_snapshot(&expired_third));
    // 升级链依赖真实时间与 escalation 窗口：第二或第三拍才可能出现 reconnect，合并断言避免与阈值漂移绑死。
    let merged_expired_later: Vec<TransportCommand> = expired_second_commands
        .iter()
        .chain(expired_third_commands.iter())
        .cloned()
        .collect();
    assert_has_connectivity_reconnect_candidate(&merged_expired_later, "transportExpiredDeadline");
    assert_latest_recovery_input_signal(
        &runtime_stats,
        "transportExpiredDeadline:transportExpiredDeadline",
    );

    harness.mark_transport_recovered(profile.baseline.now_ms + 928.0);
    let recovered_snapshot =
        harness.build_connected_snapshot(6, profile.baseline.now_ms + 930.0, 260, "none");
    let recovered_commands = transport_commands(policy.on_snapshot(&recovered_snapshot));
    assert!(
        recovered_commands.is_empty(),
        "unexpected recovered commands: {recovered_commands:?}"
    );
}

#[tokio::test]
#[ignore = "过时 replay harness：Phase C 收口前不跑 drain+policy 集成"]
async fn multi_stage_replay_steady_local_noise_sample_loss_then_recover_stays_local() {
    let repair_limit = LocalIngressReplayFixture::new(1).repair_backlog_limit();
    let profile = repair_overflow_replay_profile(repair_limit);
    let harness = run_local_ingress_replay_profile(&profile).await;
    let runtime_stats = harness.runtime_stats();
    let mut policy = RtcSessionPolicy::new(
        Arc::new(Mutex::new(
            crate::api::runtime::XbxEngineRuntimeConfig::default(),
        )),
        runtime_stats.clone(),
    );

    let steady_snapshot = harness.build_connected_snapshot(1, profile.baseline.now_ms, 240, "none");
    assert!(
        transport_commands(policy.on_snapshot(&steady_snapshot)).is_empty(),
        "unexpected steady commands"
    );

    let local_noise_snapshot =
        harness.build_connected_snapshot(2, profile.baseline.now_ms + 10.0, 241, "none");
    assert!(
        transport_commands(policy.on_snapshot(&local_noise_snapshot)).is_empty(),
        "unexpected local noise commands"
    );

    harness.mark_transport_connectivity_degraded(profile.baseline.now_ms + 30.0);
    let sample_loss = harness.build_connected_snapshot(
        3,
        profile.baseline.now_ms + 30.0,
        241,
        "transportSampleLoss",
    );
    let sample_loss_commands = transport_commands(policy.on_snapshot(&sample_loss));
    assert_no_reconnect_candidate(&sample_loss_commands);
    assert_latest_recovery_input_signal(&runtime_stats, "transportSampleLoss:transportSampleLoss");

    harness.mark_transport_recovered(profile.baseline.now_ms + 128.0);
    let recovered_snapshot =
        harness.build_connected_snapshot(4, profile.baseline.now_ms + 130.0, 260, "none");
    assert!(
        transport_commands(policy.on_snapshot(&recovered_snapshot)).is_empty(),
        "unexpected recovered commands"
    );
}

#[tokio::test]
#[ignore = "过时 replay harness：Phase C 收口前不跑 drain+policy 集成"]
async fn multi_stage_replay_steady_local_noise_recovered_late_then_recover_stays_local() {
    let repair_limit = LocalIngressReplayFixture::new(1).repair_backlog_limit();
    let profile = repair_overflow_replay_profile(repair_limit);
    let harness = run_local_ingress_replay_profile(&profile).await;
    let runtime_stats = harness.runtime_stats();
    let mut policy = RtcSessionPolicy::new(
        Arc::new(Mutex::new(
            crate::api::runtime::XbxEngineRuntimeConfig::default(),
        )),
        runtime_stats.clone(),
    );

    let steady_snapshot = harness.build_connected_snapshot(1, profile.baseline.now_ms, 240, "none");
    assert!(
        transport_commands(policy.on_snapshot(&steady_snapshot)).is_empty(),
        "unexpected steady commands"
    );

    let local_noise_snapshot =
        harness.build_connected_snapshot(2, profile.baseline.now_ms + 10.0, 241, "none");
    assert!(
        transport_commands(policy.on_snapshot(&local_noise_snapshot)).is_empty(),
        "unexpected local noise commands"
    );

    harness.mark_transport_connectivity_degraded(profile.baseline.now_ms + 30.0);
    let recovered_late = harness.build_connected_snapshot(
        3,
        profile.baseline.now_ms + 30.0,
        241,
        "transportRecoveredLate",
    );
    let recovered_late_commands = transport_commands(policy.on_snapshot(&recovered_late));
    assert_no_reconnect_candidate(&recovered_late_commands);
    assert_latest_recovery_input_signal(
        &runtime_stats,
        "transportRecoveredLate:transportRecoveredLate",
    );

    harness.mark_transport_recovered(profile.baseline.now_ms + 128.0);
    let recovered_snapshot =
        harness.build_connected_snapshot(4, profile.baseline.now_ms + 130.0, 260, "none");
    assert!(
        transport_commands(policy.on_snapshot(&recovered_snapshot)).is_empty(),
        "unexpected recovered commands"
    );
}
