use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use rtc::interceptor::{Interceptor, Packet};
use rtc::media_stream::MediaStreamTrackId;
use rtc::rtp_transceiver::RTCRtpReceiverId;
use rtc::sansio::Protocol;
use rtc::shared::marshal::Marshal;
use rtc::shared::TransportContext;
use rtc_rtcp::transport_feedbacks::transport_layer_cc::TransportLayerCc;
use rtc_rtp::extension::transport_cc_extension::TransportCcExtension;

use super::{
    build_local_twcc_interceptor, parse_twcc_binding_info_from_answer_sdp,
    ControlledTwccFeedbackController,
};
use crate::XbxEngineMediaRuntimeStats;

fn make_rtp_packet_with_twcc(
    ssrc: u32,
    seq: u16,
    twcc_seq: u16,
    hdr_ext_id: u8,
) -> rtc_rtp::packet::Packet {
    let mut pkt = rtc_rtp::packet::Packet {
        header: rtc_rtp::header::Header {
            ssrc,
            sequence_number: seq,
            payload_type: 124,
            ..Default::default()
        },
        payload: vec![0u8; 64].into(),
    };
    let ext = TransportCcExtension {
        transport_sequence: twcc_seq,
    };
    let payload = ext.marshal().unwrap();
    pkt.header
        .set_extension(hdr_ext_id, payload.freeze())
        .unwrap();
    pkt
}

#[test]
fn parse_twcc_binding_info_extracts_ext_feedback_and_codec() {
    let sdp = concat!(
        "m=video 9 UDP/TLS/RTP/SAVPF 124\r\n",
        "a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01\r\n",
        "a=rtpmap:124 H264/90000\r\n",
        "a=rtcp-fb:124 transport-cc\r\n",
    );
    let info = parse_twcc_binding_info_from_answer_sdp(sdp, 124);
    assert_eq!(info.twcc_ext_id, Some(3));
    assert_eq!(info.mime_type.as_deref(), Some("H264/90000"));
    assert!(info
        .rtcp_feedback
        .iter()
        .any(|feedback| feedback == "transport-cc:"));
}

#[test]
fn parse_twcc_binding_info_uses_matching_media_section() {
    let sdp = concat!(
        "a=extmap:9 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01\r\n",
        "m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n",
        "a=extmap:1 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01\r\n",
        "a=rtpmap:111 opus/48000/2\r\n",
        "a=rtcp-fb:111 transport-cc\r\n",
        "m=video 9 UDP/TLS/RTP/SAVPF 124\r\n",
        "a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01\r\n",
        "a=rtpmap:124 H264/90000\r\n",
        "a=rtcp-fb:124 transport-cc\r\n",
        "a=rtcp-fb:124 nack pli\r\n",
    );

    let info = parse_twcc_binding_info_from_answer_sdp(sdp, 124);

    assert_eq!(info.twcc_ext_id, Some(3));
    assert_eq!(info.mime_type.as_deref(), Some("H264/90000"));
    assert!(info.header_extensions.iter().any(|extension| extension
        == "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01#3"));
    assert!(info
        .rtcp_feedback
        .iter()
        .any(|feedback| feedback == "transport-cc:"));
    assert!(info
        .rtcp_feedback
        .iter()
        .any(|feedback| feedback == "nack:pli"));
    assert!(!info
        .header_extensions
        .iter()
        .any(|extension| extension.ends_with("#1")));
}

#[test]
fn controlled_twcc_controller_emits_local_feedback_observation() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut controller = ControlledTwccFeedbackController::new(1);
    let track_id: MediaStreamTrackId = "video".to_string();
    let ssrc = 0x22334455;
    let answer_sdp = concat!(
        "m=video 9 UDP/TLS/RTP/SAVPF 124\r\n",
        "a=extmap:5 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01\r\n",
        "a=rtpmap:124 H264/90000\r\n",
        "a=rtcp-fb:124 transport-cc\r\n",
    );

    let receiver_id = None;
    if let Some(receiver_id) = receiver_id {
        controller.register_track_open(&track_id, receiver_id);
    }

    let packet = make_rtp_packet_with_twcc(ssrc, 1, 7, 5);
    controller
        .observe_inbound_rtp(
            &track_id,
            &packet,
            &runtime_stats,
            Some(answer_sdp),
            Some("video/H264".to_string()),
        )
        .unwrap();
    assert!(controller.remote_twcc_streams.contains_key(&ssrc));
    assert!(controller.interceptor.poll_timeout().is_some());
    thread::sleep(Duration::from_millis(10));
    controller
        .interceptor
        .handle_timeout(Instant::now())
        .unwrap();
    while let Some(tagged_packet) = controller.interceptor.poll_write() {
        let Packet::Rtcp(rtcp_packets) = tagged_packet.message else {
            continue;
        };
        for packet in rtcp_packets {
            if let Some(twcc) = packet.as_any().downcast_ref::<TransportLayerCc>() {
                controller.observe_local_feedback(&runtime_stats, twcc, Some(ssrc));
            }
        }
    }

    let stats = runtime_stats.lock().unwrap();
    assert_eq!(
        stats
            .latest_video_twcc_observation
            .as_ref()
            .map(|observation| observation.source.as_str()),
        Some("local-feedback")
    );
}

#[test]
fn register_track_open_backfills_existing_video_binding() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut controller = ControlledTwccFeedbackController::new(1);
    let track_id: MediaStreamTrackId = "video".to_string();
    let ssrc = 0x33445566;
    let answer_sdp = concat!(
        "m=video 9 UDP/TLS/RTP/SAVPF 124\r\n",
        "a=extmap:5 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01\r\n",
        "a=rtpmap:124 H264/90000\r\n",
        "a=rtcp-fb:124 transport-cc\r\n",
    );
    let packet = make_rtp_packet_with_twcc(ssrc, 1, 7, 5);

    controller
        .observe_inbound_rtp(
            &track_id,
            &packet,
            &runtime_stats,
            Some(answer_sdp),
            Some("video/H264".to_string()),
        )
        .unwrap();

    let binding = controller.remote_twcc_streams.get(&ssrc).unwrap();
    assert!(binding.receiver_id.is_none());
    assert_eq!(controller.preferred_video_receiver_id, None);
    assert_eq!(controller.preferred_video_media_ssrc, Some(ssrc));

    let receiver_id = RTCRtpReceiverId::default();
    controller.register_track_open(&track_id, receiver_id);

    let binding = controller.remote_twcc_streams.get(&ssrc).unwrap();
    assert_eq!(binding.receiver_id, Some(receiver_id));
    assert_eq!(controller.preferred_video_receiver_id, Some(receiver_id));
    assert_eq!(controller.preferred_video_media_ssrc, Some(ssrc));
    assert_eq!(
        controller.preferred_video_feedback_target(),
        Some((receiver_id, Some(ssrc)))
    );
}

#[test]
fn local_twcc_interceptor_builds_feedback_after_timeout() {
    let mut interceptor = build_local_twcc_interceptor(Duration::from_millis(1));
    interceptor.bind_remote_stream(&super::build_stream_info(
        12345,
        124,
        "video/H264",
        5,
        &["transport-cc:".to_string()],
    ));
    interceptor
        .handle_read(rtc::interceptor::TaggedPacket {
            now: Instant::now(),
            transport: TransportContext::default(),
            message: Packet::Rtp(make_rtp_packet_with_twcc(12345, 1, 9, 5)),
        })
        .unwrap();
    thread::sleep(Duration::from_millis(2));
    interceptor.handle_timeout(Instant::now()).unwrap();
    let mut emitted = false;
    while let Some(packet) = interceptor.poll_write() {
        if let Packet::Rtcp(_) = packet.message {
            emitted = true;
        }
    }
    assert!(emitted);
}

#[test]
fn twcc_inbound_extension_counters_are_scoped_per_ssrc() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut controller = ControlledTwccFeedbackController::new(1);
    let track_id: MediaStreamTrackId = "video".to_string();
    let answer_sdp = concat!(
        "m=video 9 UDP/TLS/RTP/SAVPF 124\r\n",
        "a=extmap:5 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01\r\n",
        "a=rtpmap:124 H264/90000\r\n",
        "a=rtcp-fb:124 transport-cc\r\n",
    );
    let ssrc_a = 0x1001u32;
    let ssrc_b = 0x1002u32;

    let mut missing_pkt = make_rtp_packet_with_twcc(ssrc_a, 3, 11, 5);
    missing_pkt.header.extensions.clear();

    controller
        .observe_inbound_rtp(
            &track_id,
            &make_rtp_packet_with_twcc(ssrc_a, 1, 9, 5),
            &runtime_stats,
            Some(answer_sdp),
            Some("video/H264".to_string()),
        )
        .unwrap();
    controller
        .observe_inbound_rtp(
            &track_id,
            &make_rtp_packet_with_twcc(ssrc_a, 2, 10, 5),
            &runtime_stats,
            Some(answer_sdp),
            Some("video/H264".to_string()),
        )
        .unwrap();
    controller
        .observe_inbound_rtp(
            &track_id,
            &missing_pkt,
            &runtime_stats,
            Some(answer_sdp),
            Some("video/H264".to_string()),
        )
        .unwrap();
    controller
        .observe_inbound_rtp(
            &track_id,
            &make_rtp_packet_with_twcc(ssrc_b, 1, 20, 5),
            &runtime_stats,
            Some(answer_sdp),
            Some("video/H264".to_string()),
        )
        .unwrap();

    let binding_a = controller.remote_twcc_streams.get(&ssrc_a).unwrap();
    let binding_b = controller.remote_twcc_streams.get(&ssrc_b).unwrap();
    assert_eq!(binding_a.packet_seen_count, 3);
    assert_eq!(binding_a.missing_extension_count, 1);
    assert_eq!(binding_b.packet_seen_count, 1);
    assert_eq!(binding_b.missing_extension_count, 0);
}

#[test]
fn unroutable_feedback_packets_are_queued_instead_of_silently_dropped() {
    let mut controller = ControlledTwccFeedbackController::new(1);
    let mut routed = HashMap::<RTCRtpReceiverId, Vec<Box<dyn rtc::rtcp::Packet>>>::new();
    let packet: Box<dyn rtc::rtcp::Packet> = Box::new(TransportLayerCc {
        media_ssrc: 0x4455,
        ..Default::default()
    });

    controller.route_or_queue_feedback_packet(Some(0x4455), packet, &mut routed);

    assert!(routed.is_empty());
    assert_eq!(controller.pending_feedback_packets.len(), 1);
    assert_eq!(
        controller.pending_feedback_packets[0].media_ssrc,
        Some(0x4455)
    );
}
