use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rtc_rtcp::transport_feedbacks::transport_layer_cc::{
    PacketStatusChunk, StatusChunkTypeTcc, StatusVectorChunk, SymbolSizeTypeTcc,
};
use rtc_rtcp::transport_feedbacks::transport_layer_cc::{SymbolTypeTcc, TransportLayerCc};

use super::{
    build_transport_candidate_pair, build_twcc_observation,
    build_twcc_observation_with_packet_bytes, classify_transport_path,
    estimate_recent_inbound_bitrate_kbps, is_video_inbound_stream_by_hints,
    resolve_candidate_address_family, resolve_transport_address_family, resolve_transport_protocol,
    TransportAddressFamily, TWCC_OBSERVATION_SOURCE_LOCAL_FEEDBACK,
    TWCC_OBSERVATION_SOURCE_REMOTE_RTCP,
};
use crate::XbxEngineMediaRuntimeStats;
use rtc::peer_connection::transport::RTCIceCandidateType;
use rtc::rtp_transceiver::rtp_sender::RtpCodecKind;
use xbxengine_protocol::XbxEngineTargetTypeDto;

#[test]
fn relay_path_is_detected_from_either_side() {
    assert_eq!(
        classify_transport_path(
            Some(RTCIceCandidateType::Host),
            Some(RTCIceCandidateType::Relay),
        ),
        Some("Relay".to_string())
    );
    assert_eq!(
        classify_transport_path(
            Some(RTCIceCandidateType::Relay),
            Some(RTCIceCandidateType::Host),
        ),
        Some("Relay".to_string())
    );
}

#[test]
fn direct_path_is_kept_for_non_relay_pairs() {
    assert_eq!(
        classify_transport_path(
            Some(RTCIceCandidateType::Host),
            Some(RTCIceCandidateType::Srflx),
        ),
        Some("Direct (host->srflx)".to_string())
    );
}

#[test]
fn direct_path_falls_back_when_candidate_types_are_missing() {
    assert_eq!(
        classify_transport_path(None, None),
        Some("Direct".to_string())
    );
}

#[test]
fn transport_candidate_pair_is_normalized_to_lowercase() {
    assert_eq!(
        build_transport_candidate_pair(
            Some(RTCIceCandidateType::Host),
            Some(RTCIceCandidateType::Srflx)
        ),
        Some("host->srflx".to_string())
    );
}

#[test]
fn transport_protocol_prefers_single_value_when_consistent() {
    assert_eq!(
        resolve_transport_protocol(Some("UDP".to_string()), Some("udp".to_string())),
        Some("UDP".to_string())
    );
}

#[test]
fn transport_protocol_marks_mixed_sources() {
    assert_eq!(
        resolve_transport_protocol(Some("UDP".to_string()), Some("TCP".to_string())),
        Some("UDP/TCP".to_string())
    );
}

#[test]
fn transport_address_family_uses_mixed_when_local_and_remote_differ() {
    assert_eq!(
        resolve_transport_address_family(
            TransportAddressFamily::Ipv4,
            TransportAddressFamily::Ipv6
        ),
        "mixed"
    );
}

#[test]
fn transport_address_family_falls_back_to_known_side() {
    assert_eq!(
        resolve_transport_address_family(
            TransportAddressFamily::Unknown,
            TransportAddressFamily::Ipv6
        ),
        "ipv6"
    );
}

#[test]
fn candidate_address_family_detects_ipv4_ipv6_and_unknown() {
    assert_eq!(
        resolve_candidate_address_family(Some("192.168.0.2")),
        TransportAddressFamily::Ipv4
    );
    assert_eq!(
        resolve_candidate_address_family(Some("[2001:db8::1]")),
        TransportAddressFamily::Ipv6
    );
    assert_eq!(
        resolve_candidate_address_family(Some("")),
        TransportAddressFamily::Unknown
    );
}

#[test]
fn recent_inbound_bitrate_uses_window_delta_when_previous_sample_exists() {
    let bitrate = estimate_recent_inbound_bitrate_kbps(
        1_800_000,
        Some(900_000),
        Some(500.0),
        Some(1_000.0),
        2_000.0,
    );
    assert_eq!(bitrate, 7_200.0);
}

#[test]
fn recent_inbound_bitrate_falls_back_to_connection_start_on_first_sample() {
    let bitrate = estimate_recent_inbound_bitrate_kbps(900_000, None, Some(500.0), None, 1_500.0);
    assert_eq!(bitrate, 7_200.0);
}

#[test]
fn video_kind_is_accepted_even_when_frame_decode_hints_are_zero() {
    assert!(is_video_inbound_stream_by_hints(
        RtpCodecKind::Video,
        0,
        0,
        0,
        0,
        "",
    ));
}

#[test]
fn audio_kind_without_video_hints_is_rejected() {
    assert!(!is_video_inbound_stream_by_hints(
        RtpCodecKind::Audio,
        0,
        0,
        0,
        0,
        "",
    ));
}

#[test]
fn audio_kind_with_video_decode_hints_still_uses_fallback_video_detection() {
    assert!(is_video_inbound_stream_by_hints(
        RtpCodecKind::Audio,
        1920,
        1080,
        0,
        0,
        "",
    ));
}

#[test]
fn remote_rtcp_twcc_observation_does_not_fallback_to_video_transport_bitrate() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats {
        inbound_video_bitrate_kbps: Some(9_000.0),
        ..XbxEngineMediaRuntimeStats::default()
    }));
    let packet = TransportLayerCc {
        packet_status_count: 10,
        ..TransportLayerCc::default()
    };

    let observation = build_twcc_observation(
        1,
        &packet,
        &runtime_stats,
        TWCC_OBSERVATION_SOURCE_REMOTE_RTCP,
    )
    .expect("twcc observation should be built");

    assert_eq!(observation.observed_byte_count, 0);
    assert_eq!(observation.receive_bitrate_kbps, None);
}

#[test]
fn local_feedback_twcc_observation_without_ledger_is_marked_invalid() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats {
        inbound_video_bitrate_kbps: Some(9_000.0),
        ..XbxEngineMediaRuntimeStats::default()
    }));
    let packet = TransportLayerCc {
        packet_status_count: 10,
        ..TransportLayerCc::default()
    };

    let observation = build_twcc_observation(
        1,
        &packet,
        &runtime_stats,
        TWCC_OBSERVATION_SOURCE_LOCAL_FEEDBACK,
    )
    .expect("twcc observation should be built");

    assert_eq!(observation.receive_bitrate_kbps, None);
    assert_eq!(observation.coverage_ratio, Some(0.0));
    assert_eq!(observation.ledger_hit_ratio, None);
    assert!(!observation.twcc_sample_valid);
    assert!(observation
        .twcc_invalid_reason
        .as_deref()
        .unwrap_or("")
        .contains("sample-too-small"));
}

#[test]
fn first_local_feedback_twcc_uses_optimistic_delivery_ratio_when_interval_is_missing() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let packet = TransportLayerCc {
        packet_status_count: 93,
        recv_deltas: std::iter::repeat_n(
            rtc_rtcp::transport_feedbacks::transport_layer_cc::RecvDelta {
                type_tcc_packet: SymbolTypeTcc::PacketReceivedSmallDelta,
                delta: 1_000,
            },
            17,
        )
        .collect(),
        ..TransportLayerCc::default()
    };

    let observation = build_twcc_observation(
        1,
        &packet,
        &runtime_stats,
        TWCC_OBSERVATION_SOURCE_LOCAL_FEEDBACK,
    )
    .expect("twcc observation should be built");

    assert_eq!(observation.feedback_interval_ms, None);
    assert_eq!(observation.observed_packet_count, 17);
    assert_eq!(observation.covered_sequence_span, 93);
    assert!(observation.coverage_ratio.is_some());
    assert!(observation.ledger_hit_ratio.is_none());
    assert_eq!(observation.delivery_ratio, 1.0);
    assert_eq!(observation.packet_loss_ratio, 0.0);
}

#[test]
fn local_feedback_twcc_observation_uses_packet_byte_ledger_when_available() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let packet = TransportLayerCc {
        base_sequence_number: 100,
        packet_status_count: 8,
        packet_chunks: vec![PacketStatusChunk::StatusVectorChunk(StatusVectorChunk {
            type_tcc: StatusChunkTypeTcc::StatusVectorChunk,
            symbol_size: SymbolSizeTypeTcc::TwoBit,
            symbol_list: vec![
                SymbolTypeTcc::PacketReceivedSmallDelta,
                SymbolTypeTcc::PacketReceivedSmallDelta,
                SymbolTypeTcc::PacketReceivedSmallDelta,
                SymbolTypeTcc::PacketReceivedSmallDelta,
                SymbolTypeTcc::PacketReceivedSmallDelta,
                SymbolTypeTcc::PacketReceivedSmallDelta,
                SymbolTypeTcc::PacketReceivedSmallDelta,
                SymbolTypeTcc::PacketReceivedSmallDelta,
            ],
        })],
        recv_deltas: vec![
            rtc_rtcp::transport_feedbacks::transport_layer_cc::RecvDelta {
                type_tcc_packet: SymbolTypeTcc::PacketReceivedSmallDelta,
                delta: 10_000,
            },
            rtc_rtcp::transport_feedbacks::transport_layer_cc::RecvDelta {
                type_tcc_packet: SymbolTypeTcc::PacketReceivedSmallDelta,
                delta: 10_000,
            },
            rtc_rtcp::transport_feedbacks::transport_layer_cc::RecvDelta {
                type_tcc_packet: SymbolTypeTcc::PacketReceivedSmallDelta,
                delta: 10_000,
            },
            rtc_rtcp::transport_feedbacks::transport_layer_cc::RecvDelta {
                type_tcc_packet: SymbolTypeTcc::PacketReceivedSmallDelta,
                delta: 10_000,
            },
            rtc_rtcp::transport_feedbacks::transport_layer_cc::RecvDelta {
                type_tcc_packet: SymbolTypeTcc::PacketReceivedSmallDelta,
                delta: 10_000,
            },
            rtc_rtcp::transport_feedbacks::transport_layer_cc::RecvDelta {
                type_tcc_packet: SymbolTypeTcc::PacketReceivedSmallDelta,
                delta: 10_000,
            },
            rtc_rtcp::transport_feedbacks::transport_layer_cc::RecvDelta {
                type_tcc_packet: SymbolTypeTcc::PacketReceivedSmallDelta,
                delta: 10_000,
            },
            rtc_rtcp::transport_feedbacks::transport_layer_cc::RecvDelta {
                type_tcc_packet: SymbolTypeTcc::PacketReceivedSmallDelta,
                delta: 10_000,
            },
        ],
        ..TransportLayerCc::default()
    };
    let mut ledger = HashMap::new();
    ledger.insert(100, 1200);
    ledger.insert(101, 1300);
    ledger.insert(102, 1400);
    ledger.insert(103, 1500);
    ledger.insert(104, 1600);
    ledger.insert(105, 1700);
    ledger.insert(106, 1800);
    ledger.insert(107, 1900);

    let mut first_observation = build_twcc_observation_with_packet_bytes(
        1,
        &packet,
        &runtime_stats,
        TWCC_OBSERVATION_SOURCE_LOCAL_FEEDBACK,
        Some(&ledger),
    )
    .expect("twcc observation should be built");
    assert!(first_observation.twcc_sample_valid);
    assert_eq!(first_observation.observed_byte_count, 12400);
    assert_eq!(first_observation.coverage_ratio, Some(1.0));
    assert_eq!(first_observation.ledger_hit_ratio, Some(1.0));
    assert_eq!(first_observation.receive_bitrate_kbps, None);
    // 避免两次观测落在同一毫秒，导致反馈间隔偶发为 None。
    first_observation.observed_at_ms = (first_observation.observed_at_ms - 1.0).max(0.0);

    if let Ok(mut stats) = runtime_stats.lock() {
        stats.latest_video_twcc_observation = Some(first_observation);
    }

    let second_observation = build_twcc_observation_with_packet_bytes(
        2,
        &packet,
        &runtime_stats,
        TWCC_OBSERVATION_SOURCE_LOCAL_FEEDBACK,
        Some(&ledger),
    )
    .expect("twcc observation should be built");
    assert!(second_observation.twcc_sample_valid);
    assert_eq!(second_observation.observed_byte_count, 12400);
    assert_eq!(second_observation.coverage_ratio, Some(1.0));
    assert_eq!(second_observation.ledger_hit_ratio, Some(1.0));
    assert!(second_observation.feedback_interval_ms.is_some());
    assert!(second_observation.receive_bitrate_kbps.is_some());
}

#[test]
fn local_feedback_twcc_observation_marks_invalid_when_sample_interval_too_long() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let packet = TransportLayerCc {
        base_sequence_number: 100,
        packet_status_count: 8,
        packet_chunks: vec![PacketStatusChunk::StatusVectorChunk(StatusVectorChunk {
            type_tcc: StatusChunkTypeTcc::StatusVectorChunk,
            symbol_size: SymbolSizeTypeTcc::TwoBit,
            symbol_list: vec![
                SymbolTypeTcc::PacketReceivedSmallDelta,
                SymbolTypeTcc::PacketReceivedSmallDelta,
                SymbolTypeTcc::PacketReceivedSmallDelta,
                SymbolTypeTcc::PacketReceivedSmallDelta,
                SymbolTypeTcc::PacketReceivedSmallDelta,
                SymbolTypeTcc::PacketReceivedSmallDelta,
                SymbolTypeTcc::PacketReceivedSmallDelta,
                SymbolTypeTcc::PacketReceivedSmallDelta,
            ],
        })],
        recv_deltas: std::iter::repeat_n(
            rtc_rtcp::transport_feedbacks::transport_layer_cc::RecvDelta {
                type_tcc_packet: SymbolTypeTcc::PacketReceivedSmallDelta,
                delta: 80_000,
            },
            8,
        )
        .collect(),
        ..TransportLayerCc::default()
    };
    let ledger = HashMap::from([
        (100, 1200),
        (101, 1300),
        (102, 1400),
        (103, 1500),
        (104, 1600),
        (105, 1700),
        (106, 1800),
        (107, 1900),
    ]);

    let observation = build_twcc_observation_with_packet_bytes(
        1,
        &packet,
        &runtime_stats,
        TWCC_OBSERVATION_SOURCE_LOCAL_FEEDBACK,
        Some(&ledger),
    )
    .expect("twcc observation should be built");
    assert!(!observation.twcc_sample_valid);
    assert!(observation
        .twcc_invalid_reason
        .as_deref()
        .unwrap_or("")
        .contains("interval-too-long"));
    assert_eq!(observation.observed_byte_count, 12400);
    assert_eq!(observation.ledger_hit_ratio, Some(1.0));
}

#[test]
fn cloud_local_feedback_tolerates_longer_sample_interval() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
    }
    let packet = TransportLayerCc {
        base_sequence_number: 100,
        packet_status_count: 8,
        packet_chunks: vec![PacketStatusChunk::StatusVectorChunk(StatusVectorChunk {
            type_tcc: StatusChunkTypeTcc::StatusVectorChunk,
            symbol_size: SymbolSizeTypeTcc::TwoBit,
            symbol_list: vec![
                SymbolTypeTcc::PacketReceivedSmallDelta,
                SymbolTypeTcc::PacketReceivedSmallDelta,
                SymbolTypeTcc::PacketReceivedSmallDelta,
                SymbolTypeTcc::PacketReceivedSmallDelta,
                SymbolTypeTcc::PacketReceivedSmallDelta,
                SymbolTypeTcc::PacketReceivedSmallDelta,
                SymbolTypeTcc::PacketReceivedSmallDelta,
                SymbolTypeTcc::PacketReceivedSmallDelta,
            ],
        })],
        recv_deltas: std::iter::repeat_n(
            rtc_rtcp::transport_feedbacks::transport_layer_cc::RecvDelta {
                type_tcc_packet: SymbolTypeTcc::PacketReceivedSmallDelta,
                delta: 80_000,
            },
            8,
        )
        .collect(),
        ..TransportLayerCc::default()
    };
    let ledger = HashMap::from([
        (100, 1200),
        (101, 1300),
        (102, 1400),
        (103, 1500),
        (104, 1600),
        (105, 1700),
        (106, 1800),
        (107, 1900),
    ]);

    let observation = build_twcc_observation_with_packet_bytes(
        1,
        &packet,
        &runtime_stats,
        TWCC_OBSERVATION_SOURCE_LOCAL_FEEDBACK,
        Some(&ledger),
    )
    .expect("twcc observation should be built");
    assert!(observation.twcc_sample_valid);
    assert_eq!(observation.twcc_invalid_reason, None);
    assert_eq!(observation.observed_byte_count, 12400);
    assert_eq!(observation.ledger_hit_ratio, Some(1.0));
}

#[test]
fn local_feedback_twcc_observation_marks_invalid_when_coverage_too_low() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let packet = TransportLayerCc {
        base_sequence_number: 100,
        packet_status_count: 10,
        packet_chunks: vec![PacketStatusChunk::RunLengthChunk(
            rtc_rtcp::transport_feedbacks::transport_layer_cc::RunLengthChunk {
                type_tcc: StatusChunkTypeTcc::RunLengthChunk,
                packet_status_symbol: SymbolTypeTcc::PacketReceivedSmallDelta,
                run_length: 4,
            },
        )],
        recv_deltas: std::iter::repeat_n(
            rtc_rtcp::transport_feedbacks::transport_layer_cc::RecvDelta {
                type_tcc_packet: SymbolTypeTcc::PacketReceivedSmallDelta,
                delta: 10_000,
            },
            4,
        )
        .collect(),
        ..TransportLayerCc::default()
    };
    let mut ledger = HashMap::new();
    ledger.insert(100, 1200);
    ledger.insert(101, 1200);
    ledger.insert(102, 1200);
    ledger.insert(103, 1200);

    let observation = build_twcc_observation_with_packet_bytes(
        1,
        &packet,
        &runtime_stats,
        TWCC_OBSERVATION_SOURCE_LOCAL_FEEDBACK,
        Some(&ledger),
    )
    .expect("twcc observation should be built");

    assert_eq!(observation.coverage_ratio, Some(0.4));
    assert!(!observation.twcc_sample_valid);
    assert!(observation
        .twcc_invalid_reason
        .as_deref()
        .unwrap_or("")
        .contains("coverage-too-low"));
}

#[test]
fn local_feedback_twcc_observation_marks_invalid_when_ledger_hit_too_low() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let packet = TransportLayerCc {
        base_sequence_number: 100,
        packet_status_count: 10,
        packet_chunks: vec![PacketStatusChunk::RunLengthChunk(
            rtc_rtcp::transport_feedbacks::transport_layer_cc::RunLengthChunk {
                type_tcc: StatusChunkTypeTcc::RunLengthChunk,
                packet_status_symbol: SymbolTypeTcc::PacketReceivedSmallDelta,
                run_length: 10,
            },
        )],
        recv_deltas: std::iter::repeat_n(
            rtc_rtcp::transport_feedbacks::transport_layer_cc::RecvDelta {
                type_tcc_packet: SymbolTypeTcc::PacketReceivedSmallDelta,
                delta: 10_000,
            },
            10,
        )
        .collect(),
        ..TransportLayerCc::default()
    };
    let mut ledger = HashMap::new();
    ledger.insert(100, 1200);
    ledger.insert(101, 1200);
    ledger.insert(102, 1200);
    ledger.insert(103, 1200);
    ledger.insert(104, 1200);

    let observation = build_twcc_observation_with_packet_bytes(
        1,
        &packet,
        &runtime_stats,
        TWCC_OBSERVATION_SOURCE_LOCAL_FEEDBACK,
        Some(&ledger),
    )
    .expect("twcc observation should be built");

    assert_eq!(observation.coverage_ratio, Some(1.0));
    assert_eq!(observation.ledger_hit_ratio, Some(0.5));
    assert!(!observation.twcc_sample_valid);
    assert!(observation
        .twcc_invalid_reason
        .as_deref()
        .unwrap_or("")
        .contains("ledger-hit-too-low"));
}
