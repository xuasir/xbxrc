use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::thread;
use std::time::{Duration, Instant};

use bytes::BytesMut;
use rtc::peer_connection::configuration::media_engine::MediaEngine;
use rtc::peer_connection::configuration::RTCConfigurationBuilder;
use rtc::peer_connection::event::{RTCDataChannelEvent, RTCPeerConnectionEvent};
use rtc::peer_connection::sdp::RTCSessionDescription;
use rtc::peer_connection::state::RTCPeerConnectionState;
use rtc::peer_connection::transport::{
    CandidateConfig, CandidateHostConfig, RTCIceCandidate, RTCIceCandidateInit,
};
use rtc::peer_connection::RTCPeerConnection;
use rtc::peer_connection::RTCPeerConnectionBuilder;
use rtc::sansio::Protocol;
use rtc::shared::{TaggedBytesMut, TransportContext, TransportProtocol};

use super::{
    build_owned_h264_codec_preferences, register_owned_h264_codecs, RtcConnectionLifecycleState,
    RtcConnectionService,
};
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::connection::data_channel::{
    CHAT_CHANNEL_LABEL, CONTROL_CHANNEL_LABEL, INPUT_CHANNEL_LABEL, MESSAGE_CHANNEL_LABEL,
};
use crate::transport::rtc::connection::dto_to_rtc_candidate;
use crate::transport::rtc::connection::transport_metrics::publish_transport_metrics_sample;
use crate::transport::rtc::connection::transport_metrics::RtcTransportMetricsSnapshot;
use crate::{XbxEngineMediaRuntimeStats, XbxEngineRuntimeError};
use std::sync::{Arc, Mutex};
use xbxengine_protocol::{XbxEngineIceCandidateDto, XbxEngineSessionDto, XbxEngineTargetTypeDto};

const HANDSHAKE_ACK_PAYLOAD: &str = r#"{"type":"HandshakeAck"}"#;

#[test]
fn create_raw_offer_comes_from_real_rtc_peer_connection() {
    let mut service = RtcConnectionService::default();
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let session = XbxEngineSessionDto {
        session_id: "test-session".to_string(),
        target_type: XbxEngineTargetTypeDto::Cloud,
        turn_server: None,
    };

    service.rebuild(&session, &runtime_stats).unwrap();
    let offer = service
        .create_raw_offer(&Default::default(), &runtime_stats)
        .unwrap();

    assert!(offer.contains("m=audio"));
    assert!(offer.contains("m=video"));
    assert!(offer.contains("m=application"));
    assert!(offer.contains("webrtc-datachannel"));
    assert!(!service.local_candidates_snapshot().is_empty());
    let state = service.state.lock().expect("connection state");
    assert_eq!(state.local_candidate_host_count, 1);
    // 移除 eager EOC 注入后，gathering 完成由底层事件驱动，不再要求这里立即完成。
    assert!(state.local_candidate_end_of_candidates_count <= 1);
    drop(state);
    assert!(runtime_stats
        .lock()
        .expect("runtime stats")
        .latest_observation_summary
        .as_deref()
        .is_some_and(|summary| summary.contains("local total=1")));
}

#[test]
fn local_candidates_snapshot_falls_back_to_offer_sdp_candidates() {
    let mut service = RtcConnectionService::default();
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let session = XbxEngineSessionDto {
        session_id: "test-session".to_string(),
        target_type: XbxEngineTargetTypeDto::Cloud,
        turn_server: None,
    };

    service.rebuild(&session, &runtime_stats).unwrap();
    let offer = service
        .create_raw_offer(&Default::default(), &runtime_stats)
        .unwrap();
    {
        let mut state = service.state.lock().expect("connection state");
        state.local_candidates.clear();
        state.local_candidate_keys.clear();
        state.local_candidate_count_total = 0;
        state.local_candidate_host_count = 0;
        state.local_candidate_srflx_count = 0;
        state.local_candidate_relay_count = 0;
        state.local_candidate_unknown_count = 0;
        state.latest_local_candidate_kind = None;
        state.latest_local_candidate_key = None;
        state.local_ice_gathering_complete = false;
        state.local_offer_sdp = Some(offer);
    }

    let candidates = service.local_candidates_snapshot();

    assert!(!candidates.is_empty());
    assert!(candidates
        .iter()
        .all(|candidate| candidate.candidate.starts_with("candidate:")));
    assert_eq!(
        service
            .state
            .lock()
            .expect("connection state")
            .local_candidate_count_total,
        candidates.len() as u64
    );
}

#[test]
fn owned_h264_codec_preferences_include_main_profile_and_rtx_probe() {
    let codecs = build_owned_h264_codec_preferences();
    assert!(codecs.iter().any(|codec| {
        codec.payload_type == 124
            && codec.rtp_codec.mime_type.eq_ignore_ascii_case("video/h264")
            && codec
                .rtp_codec
                .sdp_fmtp_line
                .contains("profile-level-id=4d0032")
    }));
    assert!(codecs.iter().any(|codec| {
        codec.payload_type == 97
            && codec.rtp_codec.mime_type.eq_ignore_ascii_case("video/rtx")
            && codec.rtp_codec.sdp_fmtp_line == "apt=124"
    }));
}

#[test]
fn register_owned_h264_codecs_is_compatible_with_default_registry() {
    let mut media_engine = MediaEngine::default();
    media_engine.register_default_codecs().unwrap();
    // rtc::MediaEngine 的 codec 列表对外不可见；这里至少保证补充注册不会报错。
    assert!(register_owned_h264_codecs(&mut media_engine).is_ok());
}

#[test]
fn refresh_transport_metrics_publishes_raw_sample_only() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    runtime_stats.lock().unwrap().video_remb_bps = Some(42_000_000);
    let sink = RuntimeStatsSink::new(runtime_stats.clone());
    let snapshot = RtcTransportMetricsSnapshot {
        video_rtt_ms: Some(48.0),
        video_rtt_source: Some("candidate-pair".to_string()),
        inbound_video_loss_ratio_5s: 0.0,
        inbound_video_loss_ratio_1s: 0.0,
        transport_path: Some("Direct (host->host)".to_string()),
        inbound_video_bitrate_kbps: 11_500.0,
        inbound_primary_video_bytes_total: 900_000,
    };

    publish_transport_metrics_sample(&sink, &snapshot);

    let stats = runtime_stats.lock().unwrap();
    assert_eq!(stats.video_rtt_ms, Some(48.0));
    assert_eq!(stats.video_rtt_source.as_deref(), Some("candidate-pair"));
    assert_eq!(stats.inbound_video_loss_ratio_1s, 0.0);
    assert_eq!(stats.inbound_video_bitrate_kbps, Some(11_500.0));
    assert_eq!(stats.inbound_primary_video_bytes_total, 900_000);
    assert_eq!(stats.video_remb_bps, Some(42_000_000));
    assert!(stats.latest_video_bwe_observation.is_none());
}

#[test]
fn apply_remote_description_accepts_real_rtc_answer() {
    let mut service = RtcConnectionService::default();
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let session = XbxEngineSessionDto {
        session_id: "test-session".to_string(),
        target_type: XbxEngineTargetTypeDto::Home,
        turn_server: None,
    };
    service.rebuild(&session, &runtime_stats).unwrap();
    let offer = service
        .create_raw_offer(&Default::default(), &runtime_stats)
        .unwrap();

    let mut answer_pc = RTCPeerConnectionBuilder::new()
        .with_configuration(RTCConfigurationBuilder::new().build())
        .build()
        .unwrap();
    answer_pc
        .set_remote_description(RTCSessionDescription::offer(offer).unwrap())
        .unwrap();
    let answer = answer_pc.create_answer(None).unwrap();
    answer_pc.set_local_description(answer.clone()).unwrap();

    service
        .apply_remote_description(&answer.sdp, &[], &runtime_stats)
        .unwrap();
}

#[test]
fn add_remote_ice_candidates_deduplicates_when_remote_description_missing() {
    let mut service = RtcConnectionService::default();
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let session = XbxEngineSessionDto {
        session_id: "test-session".to_string(),
        target_type: XbxEngineTargetTypeDto::Cloud,
        turn_server: None,
    };
    service.rebuild(&session, &runtime_stats).unwrap();
    let duplicate = service
        .local_candidates_snapshot()
        .into_iter()
        .next()
        .expect("service local candidate");

    service
        .add_remote_ice_candidates(&[duplicate.clone(), duplicate], &runtime_stats)
        .unwrap();

    let state = service.state.lock().expect("connection state");
    assert_eq!(state.remote_candidates.len(), 1);
    assert_eq!(state.pending_remote_candidates.len(), 1);
    assert_eq!(state.remote_candidate_keys.len(), 1);
    assert_eq!(state.pending_remote_candidate_keys.len(), 1);
}

#[test]
fn apply_remote_description_deduplicates_pending_and_inline_candidates() {
    let mut service = RtcConnectionService::default();
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let session = XbxEngineSessionDto {
        session_id: "test-session".to_string(),
        target_type: XbxEngineTargetTypeDto::Cloud,
        turn_server: None,
    };

    service.rebuild(&session, &runtime_stats).unwrap();
    let offer = service
        .create_raw_offer(&Default::default(), &runtime_stats)
        .unwrap();
    let duplicate = service
        .local_candidates_snapshot()
        .into_iter()
        .next()
        .expect("service local candidate");
    service
        .add_remote_ice_candidates(std::slice::from_ref(&duplicate), &runtime_stats)
        .unwrap();

    let mut answer_pc = RTCPeerConnectionBuilder::new()
        .with_configuration(RTCConfigurationBuilder::new().build())
        .build()
        .unwrap();
    answer_pc
        .set_remote_description(RTCSessionDescription::offer(offer).unwrap())
        .unwrap();
    let answer = answer_pc.create_answer(None).unwrap();
    answer_pc.set_local_description(answer.clone()).unwrap();

    service
        .apply_remote_description(&answer.sdp, &[duplicate.clone(), duplicate], &runtime_stats)
        .unwrap();

    let state = service.state.lock().expect("connection state");
    assert!(state.pending_remote_candidates.is_empty());
    assert!(state.pending_remote_candidate_keys.is_empty());
    assert_eq!(state.remote_candidate_keys.len(), 1);
    assert_eq!(state.applied_remote_candidate_keys.len(), 1);
}

#[test]
fn add_remote_ice_candidates_handles_out_of_order_duplicates_late_trickle_and_eoc() {
    let mut service = RtcConnectionService::default();
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let session = XbxEngineSessionDto {
        session_id: "test-session".to_string(),
        target_type: XbxEngineTargetTypeDto::Cloud,
        turn_server: None,
    };

    service.rebuild(&session, &runtime_stats).unwrap();
    let offer = service
        .create_raw_offer(&Default::default(), &runtime_stats)
        .unwrap();

    let remote_io = TestRtcPeerIo::bind().unwrap();
    let remote_candidate = remote_io.local_candidate().unwrap();
    let host = candidate_dto(
        &remote_candidate.candidate,
        remote_candidate.sdp_mid.as_deref(),
        remote_candidate.sdp_mline_index,
    );
    let srflx = candidate_dto(
        &remote_candidate.candidate.replace(
            " typ host",
            &format!(
                " typ srflx raddr {} rport {}",
                remote_io.local_addr.ip(),
                remote_io.local_addr.port()
            ),
        ),
        remote_candidate.sdp_mid.as_deref(),
        remote_candidate.sdp_mline_index,
    );
    let relay = candidate_dto(
        &remote_candidate.candidate.replace(
            " typ host",
            &format!(
                " typ relay raddr {} rport {}",
                remote_io.local_addr.ip(),
                remote_io.local_addr.port()
            ),
        ),
        remote_candidate.sdp_mid.as_deref(),
        remote_candidate.sdp_mline_index,
    );

    service
        .add_remote_ice_candidates(&[relay.clone(), host.clone(), host.clone()], &runtime_stats)
        .unwrap();
    service
        .add_remote_ice_candidates(&[candidate_dto("", Some("0"), Some(0))], &runtime_stats)
        .unwrap();

    let mut answer_pc = RTCPeerConnectionBuilder::new()
        .with_configuration(RTCConfigurationBuilder::new().build())
        .build()
        .unwrap();
    answer_pc
        .set_remote_description(RTCSessionDescription::offer(offer).unwrap())
        .unwrap();
    let answer = answer_pc.create_answer(None).unwrap();
    answer_pc.set_local_description(answer.clone()).unwrap();

    service
        .apply_remote_description(
            &answer.sdp,
            &[relay.clone(), srflx.clone(), host.clone(), srflx.clone()],
            &runtime_stats,
        )
        .unwrap();

    let state = service.state.lock().expect("connection state");
    assert!(state.pending_remote_candidates.is_empty());
    assert!(state.pending_remote_candidate_keys.is_empty());
    assert!(state.remote_ice_gathering_complete);
    assert_eq!(state.remote_candidates.len(), 3);
    assert_eq!(state.remote_candidate_keys.len(), 3);
    assert_eq!(state.applied_remote_candidate_keys.len(), 3);
    assert_eq!(state.remote_candidate_host_count, 1);
    assert_eq!(state.remote_candidate_srflx_count, 1);
    assert_eq!(state.remote_candidate_relay_count, 1);
    assert_eq!(state.remote_candidate_unknown_count, 0);
    drop(state);
    assert!(runtime_stats
        .lock()
        .expect("runtime stats")
        .latest_observation_summary
        .as_deref()
        .is_some_and(|summary| summary.contains("remote total=3")));
}

#[test]
fn service_connects_to_raw_rtc_answer_peer_and_opens_control_channel() {
    let mut service = RtcConnectionService::default();
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let session = XbxEngineSessionDto {
        session_id: "test-session".to_string(),
        target_type: XbxEngineTargetTypeDto::Cloud,
        turn_server: None,
    };

    service.rebuild(&session, &runtime_stats).unwrap();
    let offer = service
        .create_raw_offer(&Default::default(), &runtime_stats)
        .unwrap();
    let service_candidates = service.local_candidates_snapshot();
    assert!(
        !service_candidates.is_empty(),
        "service local candidates should not be empty"
    );

    let mut answer_pc = RTCPeerConnectionBuilder::new()
        .with_configuration(RTCConfigurationBuilder::new().build())
        .build()
        .unwrap();
    let mut answer_io = TestRtcPeerIo::bind().unwrap();
    let answer_candidate = answer_io.local_candidate().unwrap();

    answer_pc
        .set_remote_description(RTCSessionDescription::offer(offer).unwrap())
        .unwrap();
    answer_pc
        .add_local_candidate(answer_candidate.clone())
        .unwrap();
    answer_pc
        .add_local_candidate(end_of_candidates_for_test())
        .unwrap();
    for candidate in &service_candidates {
        answer_pc
            .add_remote_candidate(dto_to_rtc_candidate(candidate))
            .unwrap();
    }
    let answer = answer_pc.create_answer(None).unwrap();
    answer_pc.set_local_description(answer.clone()).unwrap();

    service
        .apply_remote_description(
            &answer.sdp,
            &[XbxEngineIceCandidateDto {
                candidate: answer_candidate.candidate.clone(),
                sdp_m_line_index: answer_candidate.sdp_mline_index,
                sdp_mid: answer_candidate.sdp_mid.clone(),
            }],
            &runtime_stats,
        )
        .unwrap();

    let mut answer_connected = false;
    let mut answer_message_dc_id = None;
    let mut answer_control_dc_id = None;
    let mut answer_input_dc_id = None;
    let mut answer_chat_dc_id = None;
    let mut handshake_ack_sent = false;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        service.pump(&runtime_stats).unwrap();
        answer_io.pump(&mut answer_pc).unwrap();
        while let Some(event) = answer_pc.poll_event() {
            match event {
                RTCPeerConnectionEvent::OnConnectionStateChangeEvent(
                    RTCPeerConnectionState::Connected,
                ) => {
                    answer_connected = true;
                }
                RTCPeerConnectionEvent::OnConnectionStateChangeEvent(
                    RTCPeerConnectionState::Failed,
                ) => panic!("answer peer connection failed"),
                RTCPeerConnectionEvent::OnDataChannel(RTCDataChannelEvent::OnOpen(dc_id)) => {
                    let label = answer_pc
                        .data_channel(dc_id)
                        .expect("answer data channel")
                        .label()
                        .to_string();
                    match label.as_str() {
                        MESSAGE_CHANNEL_LABEL => {
                            let _ = answer_message_dc_id.get_or_insert(dc_id);
                        }
                        CONTROL_CHANNEL_LABEL => {
                            let _ = answer_control_dc_id.get_or_insert(dc_id);
                        }
                        INPUT_CHANNEL_LABEL => {
                            let _ = answer_input_dc_id.get_or_insert(dc_id);
                        }
                        CHAT_CHANNEL_LABEL => {
                            let _ = answer_chat_dc_id.get_or_insert(dc_id);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        while let Some(message) = answer_pc.poll_read() {
            if let rtc::peer_connection::message::RTCMessage::DataChannelMessage(
                channel_id,
                payload,
            ) = message
            {
                let body = String::from_utf8_lossy(payload.data.as_ref()).to_string();
                if body.contains("\"type\":\"Handshake\"") {
                    let mut answer_dc = answer_pc
                        .data_channel(channel_id)
                        .expect("answer data channel");
                    answer_dc
                        .send_text(HANDSHAKE_ACK_PAYLOAD.to_string())
                        .unwrap();
                    handshake_ack_sent = true;
                }
            }
        }
        if handshake_ack_sent {
            answer_io.pump(&mut answer_pc).unwrap();
            service.pump(&runtime_stats).unwrap();
            answer_io.pump(&mut answer_pc).unwrap();
            handshake_ack_sent = false;
        }

        if answer_connected
            && answer_message_dc_id.is_some()
            && answer_control_dc_id.is_some()
            && answer_input_dc_id.is_some()
            && answer_chat_dc_id.is_some()
        {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    assert!(answer_connected, "answer peer should reach connected");
    assert!(
        answer_message_dc_id.is_some(),
        "message channel should open"
    );
    assert!(
        answer_control_dc_id.is_some(),
        "control channel should open"
    );
    assert!(answer_input_dc_id.is_some(), "input channel should open");
    assert!(answer_chat_dc_id.is_some(), "chat channel should open");
    assert_eq!(
        runtime_stats.lock().unwrap().transport_state,
        xbxengine_protocol::XbxEngineTransportStateDto::Connected
    );
}

#[test]
fn service_pump_observes_data_channel_message_from_poll_read() {
    let mut service = RtcConnectionService::default();
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let session = XbxEngineSessionDto {
        session_id: "test-session".to_string(),
        target_type: XbxEngineTargetTypeDto::Cloud,
        turn_server: None,
    };

    service.rebuild(&session, &runtime_stats).unwrap();
    let offer = service
        .create_raw_offer(&Default::default(), &runtime_stats)
        .unwrap();
    let service_candidates = service.local_candidates_snapshot();
    assert!(
        !service_candidates.is_empty(),
        "service local candidates should not be empty"
    );

    let mut answer_pc = RTCPeerConnectionBuilder::new()
        .with_configuration(RTCConfigurationBuilder::new().build())
        .build()
        .unwrap();
    let mut answer_io = TestRtcPeerIo::bind().unwrap();
    let answer_candidate = answer_io.local_candidate().unwrap();

    answer_pc
        .set_remote_description(RTCSessionDescription::offer(offer).unwrap())
        .unwrap();
    answer_pc
        .add_local_candidate(answer_candidate.clone())
        .unwrap();
    answer_pc
        .add_local_candidate(end_of_candidates_for_test())
        .unwrap();
    for candidate in &service_candidates {
        answer_pc
            .add_remote_candidate(dto_to_rtc_candidate(candidate))
            .unwrap();
    }
    let answer = answer_pc.create_answer(None).unwrap();
    answer_pc.set_local_description(answer.clone()).unwrap();

    service
        .apply_remote_description(
            &answer.sdp,
            &[XbxEngineIceCandidateDto {
                candidate: answer_candidate.candidate.clone(),
                sdp_m_line_index: answer_candidate.sdp_mline_index,
                sdp_mid: answer_candidate.sdp_mid.clone(),
            }],
            &runtime_stats,
        )
        .unwrap();

    let mut answer_connected = false;
    let mut answer_message_dc_id = None;
    let mut answer_control_dc_id = None;
    let mut answer_input_dc_id = None;
    let mut answer_chat_dc_id = None;
    let mut handshake_ack_sent = false;
    let mut saw_input_metadata = false;
    let connect_deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < connect_deadline {
        service.pump(&runtime_stats).unwrap();
        answer_io.pump(&mut answer_pc).unwrap();
        while let Some(event) = answer_pc.poll_event() {
            match event {
                RTCPeerConnectionEvent::OnConnectionStateChangeEvent(
                    RTCPeerConnectionState::Connected,
                ) => {
                    answer_connected = true;
                }
                RTCPeerConnectionEvent::OnDataChannel(RTCDataChannelEvent::OnOpen(dc_id)) => {
                    let label = answer_pc
                        .data_channel(dc_id)
                        .expect("answer data channel")
                        .label()
                        .to_string();
                    match label.as_str() {
                        MESSAGE_CHANNEL_LABEL => {
                            let _ = answer_message_dc_id.get_or_insert(dc_id);
                        }
                        CONTROL_CHANNEL_LABEL => {
                            let _ = answer_control_dc_id.get_or_insert(dc_id);
                        }
                        INPUT_CHANNEL_LABEL => {
                            let _ = answer_input_dc_id.get_or_insert(dc_id);
                        }
                        CHAT_CHANNEL_LABEL => {
                            let _ = answer_chat_dc_id.get_or_insert(dc_id);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        while let Some(message) = answer_pc.poll_read() {
            if let rtc::peer_connection::message::RTCMessage::DataChannelMessage(
                channel_id,
                payload,
            ) = message
            {
                let label = answer_pc
                    .data_channel(channel_id)
                    .expect("answer data channel")
                    .label()
                    .to_string();
                let body = String::from_utf8_lossy(payload.data.as_ref()).to_string();
                if label == INPUT_CHANNEL_LABEL && !payload.is_string {
                    let bytes = payload.data.as_ref();
                    saw_input_metadata =
                        bytes.len() == 15 && u16::from_le_bytes([bytes[0], bytes[1]]) == 8;
                }
                if label == MESSAGE_CHANNEL_LABEL && body.contains("\"type\":\"Handshake\"") {
                    let mut answer_dc = answer_pc
                        .data_channel(channel_id)
                        .expect("answer message channel");
                    answer_dc
                        .send_text(HANDSHAKE_ACK_PAYLOAD.to_string())
                        .unwrap();
                    handshake_ack_sent = true;
                }
            }
        }
        if handshake_ack_sent {
            service.pump(&runtime_stats).unwrap();
            answer_io.pump(&mut answer_pc).unwrap();
            handshake_ack_sent = false;
        }
        if answer_connected
            && service.control_service.is_control_ready()
            && answer_message_dc_id.is_some()
            && answer_control_dc_id.is_some()
            && answer_input_dc_id.is_some()
            && answer_chat_dc_id.is_some()
        {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    assert!(answer_connected, "answer peer should reach connected");
    assert!(
        answer_message_dc_id.is_some(),
        "message channel should open"
    );
    assert!(
        answer_control_dc_id.is_some(),
        "control channel should open"
    );
    assert!(answer_input_dc_id.is_some(), "input channel should open");
    assert!(answer_chat_dc_id.is_some(), "chat channel should open");
    assert!(
        service.control_service.is_control_ready(),
        "service control channel should become ready"
    );

    let input_dc_id = answer_input_dc_id.expect("answer input channel id");
    let input_deadline = Instant::now() + Duration::from_secs(3);
    let mut saw_input_metadata = saw_input_metadata;
    while Instant::now() < input_deadline {
        service.pump(&runtime_stats).unwrap();
        answer_io.pump(&mut answer_pc).unwrap();
        while let Some(message) = answer_pc.poll_read() {
            if let rtc::peer_connection::message::RTCMessage::DataChannelMessage(
                channel_id,
                payload,
            ) = message
            {
                let label = answer_pc
                    .data_channel(channel_id)
                    .expect("answer data channel")
                    .label()
                    .to_string();
                if label == INPUT_CHANNEL_LABEL && !payload.is_string {
                    let bytes = payload.data.as_ref();
                    saw_input_metadata =
                        bytes.len() == 15 && u16::from_le_bytes([bytes[0], bytes[1]]) == 8;
                }
            }
        }
        if saw_input_metadata {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        saw_input_metadata,
        "service should send input metadata bootstrap"
    );

    let chat_dc_id = answer_chat_dc_id.expect("answer chat channel id");
    let chat_payload = "hello from rtc chat";
    {
        let mut answer_chat_dc = answer_pc
            .data_channel(chat_dc_id)
            .expect("answer chat channel available");
        answer_chat_dc.send_text(chat_payload.to_string()).unwrap();
    }

    let chat_deadline = Instant::now() + Duration::from_secs(3);
    let mut saw_chat_catalog = false;
    while Instant::now() < chat_deadline {
        service.pump(&runtime_stats).unwrap();
        answer_io.pump(&mut answer_pc).unwrap();
        if let Ok(stats) = runtime_stats.lock() {
            if let Some(observation) = stats
                .latest_data_channel_message_catalog_observation
                .as_ref()
            {
                saw_chat_catalog = observation.channel == "chat"
                    && observation.kind_message.as_deref() == Some("text")
                    && observation.payload_len == chat_payload.len();
                if saw_chat_catalog
                    && stats.latest_observation_label.as_deref() == Some("rtcChatTextObserved")
                {
                    break;
                }
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(saw_chat_catalog, "service should catalog inbound chat text");

    {
        let mut answer_chat_dc = answer_pc
            .data_channel(chat_dc_id)
            .expect("answer chat channel available");
        answer_chat_dc.close().unwrap();
    }
    let chat_close_deadline = Instant::now() + Duration::from_secs(3);
    let mut saw_chat_closed = false;
    while Instant::now() < chat_close_deadline {
        service.pump(&runtime_stats).unwrap();
        answer_io.pump(&mut answer_pc).unwrap();
        if runtime_stats.lock().ok().is_some_and(|stats| {
            stats.latest_observation_label.as_deref() == Some("rtcChatChannelClosed")
        }) {
            saw_chat_closed = true;
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(saw_chat_closed, "service should observe chat close");

    {
        let mut answer_input_dc = answer_pc
            .data_channel(input_dc_id)
            .expect("answer input channel available");
        answer_input_dc.close().unwrap();
    }
    let input_close_deadline = Instant::now() + Duration::from_secs(3);
    let mut saw_input_closed = false;
    while Instant::now() < input_close_deadline {
        service.pump(&runtime_stats).unwrap();
        answer_io.pump(&mut answer_pc).unwrap();
        if service
            .state
            .lock()
            .ok()
            .is_some_and(|state| !state.input_channel_open)
            && runtime_stats.lock().ok().is_some_and(|stats| {
                stats.latest_observation_label.as_deref() == Some("rtcInputChannelClosed")
            })
        {
            saw_input_closed = true;
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(saw_input_closed, "service should observe input close");

    assert!(service.request_video_keyframe(&runtime_stats).is_ok());
}

#[test]
fn service_bootstraps_message_and_control_payloads_after_handshake_ack() {
    let mut service = RtcConnectionService::default();
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let session = XbxEngineSessionDto {
        session_id: "test-session".to_string(),
        target_type: XbxEngineTargetTypeDto::Cloud,
        turn_server: None,
    };

    service.rebuild(&session, &runtime_stats).unwrap();
    let offer = service
        .create_raw_offer(&Default::default(), &runtime_stats)
        .unwrap();
    let service_candidates = service.local_candidates_snapshot();
    assert!(
        !service_candidates.is_empty(),
        "service local candidates should not be empty"
    );

    let mut answer_pc = RTCPeerConnectionBuilder::new()
        .with_configuration(RTCConfigurationBuilder::new().build())
        .build()
        .unwrap();
    let mut answer_io = TestRtcPeerIo::bind().unwrap();
    let answer_candidate = answer_io.local_candidate().unwrap();

    answer_pc
        .set_remote_description(RTCSessionDescription::offer(offer).unwrap())
        .unwrap();
    answer_pc
        .add_local_candidate(answer_candidate.clone())
        .unwrap();
    answer_pc
        .add_local_candidate(end_of_candidates_for_test())
        .unwrap();
    for candidate in &service_candidates {
        answer_pc
            .add_remote_candidate(dto_to_rtc_candidate(candidate))
            .unwrap();
    }
    let answer = answer_pc.create_answer(None).unwrap();
    answer_pc.set_local_description(answer.clone()).unwrap();

    service
        .apply_remote_description(
            &answer.sdp,
            &[XbxEngineIceCandidateDto {
                candidate: answer_candidate.candidate.clone(),
                sdp_m_line_index: answer_candidate.sdp_mline_index,
                sdp_mid: answer_candidate.sdp_mid.clone(),
            }],
            &runtime_stats,
        )
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut answer_connected = false;
    let mut answer_message_dc_id = None;
    let mut answer_control_dc_id = None;
    let mut answer_input_dc_id = None;
    let mut answer_chat_dc_id = None;
    let mut saw_post_handshake = false;
    let mut saw_control_authorization = false;
    let mut saw_control_removed = false;
    let mut saw_keyframe_request = false;
    let mut saw_input_metadata = false;
    let mut saw_chat_catalog = false;
    let mut observed_message_payloads = Vec::new();

    while Instant::now() < deadline {
        service.pump(&runtime_stats).unwrap();
        answer_io.pump(&mut answer_pc).unwrap();
        while let Some(event) = answer_pc.poll_event() {
            match event {
                RTCPeerConnectionEvent::OnConnectionStateChangeEvent(
                    RTCPeerConnectionState::Connected,
                ) => {
                    answer_connected = true;
                }
                RTCPeerConnectionEvent::OnDataChannel(RTCDataChannelEvent::OnOpen(dc_id)) => {
                    let label = answer_pc
                        .data_channel(dc_id)
                        .expect("answer data channel")
                        .label()
                        .to_string();
                    match label.as_str() {
                        MESSAGE_CHANNEL_LABEL => {
                            let _ = answer_message_dc_id.get_or_insert(dc_id);
                        }
                        CONTROL_CHANNEL_LABEL => {
                            let _ = answer_control_dc_id.get_or_insert(dc_id);
                        }
                        INPUT_CHANNEL_LABEL => {
                            let _ = answer_input_dc_id.get_or_insert(dc_id);
                        }
                        CHAT_CHANNEL_LABEL => {
                            let _ = answer_chat_dc_id.get_or_insert(dc_id);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        while let Some(message) = answer_pc.poll_read() {
            if let rtc::peer_connection::message::RTCMessage::DataChannelMessage(
                channel_id,
                payload,
            ) = message
            {
                let label = answer_pc
                    .data_channel(channel_id)
                    .expect("answer data channel")
                    .label()
                    .to_string();
                let body = String::from_utf8_lossy(payload.data.as_ref()).to_string();
                if label == MESSAGE_CHANNEL_LABEL {
                    observed_message_payloads.push(body.clone());
                }
                if label == MESSAGE_CHANNEL_LABEL && body.contains("\"type\":\"Handshake\"") {
                    let mut answer_dc = answer_pc
                        .data_channel(channel_id)
                        .expect("answer message channel");
                    answer_dc
                        .send_text(HANDSHAKE_ACK_PAYLOAD.to_string())
                        .unwrap();
                }
                if label == INPUT_CHANNEL_LABEL && !payload.is_string {
                    let bytes = payload.data.as_ref();
                    saw_input_metadata =
                        bytes.len() == 15 && u16::from_le_bytes([bytes[0], bytes[1]]) == 8;
                }
                if label == CHAT_CHANNEL_LABEL && payload.is_string {
                    saw_chat_catalog = body.contains("hello from rtc chat");
                }
                if label == CONTROL_CHANNEL_LABEL
                    && body.contains("\"message\":\"videoKeyframeRequested\"")
                {
                    saw_keyframe_request = true;
                }
                if body.contains("/streaming/systemUi/configuration")
                    || body.contains("/streaming/properties/clientappinstallidchanged")
                {
                    saw_post_handshake = true;
                }
                if body.contains("\"message\":\"authorizationRequest\"") {
                    saw_control_authorization = true;
                }
                if body.contains("\"message\":\"gamepadChanged\"")
                    && body.contains("\"wasAdded\":false")
                {
                    saw_control_removed = true;
                }
            }
        }

        if service.control_service.is_control_ready() && !saw_keyframe_request {
            service.request_video_keyframe(&runtime_stats).unwrap();
        }

        if answer_connected
            && service.control_service.is_control_ready()
            && answer_message_dc_id.is_some()
            && answer_control_dc_id.is_some()
            && answer_input_dc_id.is_some()
            && answer_chat_dc_id.is_some()
            && saw_post_handshake
            && saw_control_authorization
            && saw_control_removed
            && saw_keyframe_request
        {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    assert!(answer_connected, "answer peer should reach connected");
    assert!(
        answer_message_dc_id.is_some(),
        "message channel should open"
    );
    assert!(
        answer_control_dc_id.is_some(),
        "control channel should open"
    );
    assert!(answer_input_dc_id.is_some(), "input channel should open");
    assert!(answer_chat_dc_id.is_some(), "chat channel should open");
    assert!(
        saw_post_handshake,
        "service should send post-handshake message payload, observed message payloads: {observed_message_payloads:?}"
    );
    assert!(
        saw_control_authorization,
        "service should send control authorization payload"
    );
    assert!(
        saw_control_removed,
        "service should send control gamepad removed payload"
    );
    assert!(
        saw_keyframe_request,
        "service should send keyframe request after control becomes ready"
    );

    let chat_payload = "hello from rtc chat";
    {
        let mut answer_chat_dc = answer_pc
            .data_channel(answer_chat_dc_id.expect("answer chat channel id"))
            .expect("answer chat channel available");
        answer_chat_dc.send_text(chat_payload.to_string()).unwrap();
    }

    let chat_deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < chat_deadline {
        service.pump(&runtime_stats).unwrap();
        answer_io.pump(&mut answer_pc).unwrap();
        while let Some(message) = answer_pc.poll_read() {
            if let rtc::peer_connection::message::RTCMessage::DataChannelMessage(
                channel_id,
                payload,
            ) = message
            {
                let label = answer_pc
                    .data_channel(channel_id)
                    .expect("answer data channel")
                    .label()
                    .to_string();
                if label == INPUT_CHANNEL_LABEL && !payload.is_string {
                    let bytes = payload.data.as_ref();
                    saw_input_metadata =
                        bytes.len() == 15 && u16::from_le_bytes([bytes[0], bytes[1]]) == 8;
                }
            }
        }
        if let Ok(stats) = runtime_stats.lock() {
            if let Some(observation) = stats
                .latest_data_channel_message_catalog_observation
                .as_ref()
            {
                saw_chat_catalog = observation.channel == "chat"
                    && observation.kind_message.as_deref() == Some("text")
                    && observation.payload_len == chat_payload.len();
            }
        }
        if saw_input_metadata && saw_chat_catalog {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        saw_input_metadata,
        "service should send input metadata bootstrap"
    );
    assert!(saw_chat_catalog, "service should catalog inbound chat text");

    assert!(service.request_video_keyframe(&runtime_stats).is_ok());
}

#[test]
fn service_pump_error_marks_recovering_transport_state() {
    let mut service = RtcConnectionService::default();
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let session = XbxEngineSessionDto {
        session_id: "test-session".to_string(),
        target_type: XbxEngineTargetTypeDto::Cloud,
        turn_server: None,
    };

    service.rebuild(&session, &runtime_stats).unwrap();
    let _ = service
        .create_raw_offer(&Default::default(), &runtime_stats)
        .unwrap();

    service.inject_pump_failure();

    let error = service
        .pump(&runtime_stats)
        .expect_err("pump should fail after local address fault injection");
    assert!(
        error
            .to_string()
            .contains("xbxEngineRtcPumpInjectedFailure"),
        "unexpected pump error: {error}"
    );
    assert!(
        matches!(
            service.lifecycle_state,
            RtcConnectionLifecycleState::Failed | RtcConnectionLifecycleState::Recovering
        ),
        "service should enter failed/recovering lifecycle"
    );
    assert!(matches!(
        runtime_stats.lock().unwrap().transport_state,
        xbxengine_protocol::XbxEngineTransportStateDto::Failed
            | xbxengine_protocol::XbxEngineTransportStateDto::Connecting
    ));
}

#[test]
fn schedule_immediate_reconnect_publishes_reason_and_observation_id() {
    let mut service = RtcConnectionService::default();
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));

    service.schedule_immediate_reconnect(
        &runtime_stats,
        "testReconnect",
        "test reconnect summary",
        "peer connection failed",
    );

    let stats = runtime_stats.lock().expect("runtime stats");
    assert_eq!(
        stats.latest_observation_label.as_deref(),
        Some("testReconnect")
    );
    let summary = stats
        .latest_observation_summary
        .as_deref()
        .expect("latest observation summary");
    assert!(summary.contains("reason=peer connection failed"));
    assert!(summary.contains(&format!(
        "observationId={}",
        service.lifecycle_observation_id
    )));
}

#[test]
fn mark_recovering_from_fault_publishes_recovering_summary() {
    let mut service = RtcConnectionService::default();
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));

    service.mark_recovering_from_fault(
        &runtime_stats,
        "faultLabel",
        "fault summary",
        RtcConnectionLifecycleState::Failed,
        "new reason".to_string(),
    );

    let stats = runtime_stats.lock().expect("runtime stats");
    assert_eq!(
        stats.latest_observation_label.as_deref(),
        Some("faultLabel")
    );
    let summary = stats
        .latest_observation_summary
        .as_deref()
        .expect("latest observation summary");
    assert!(summary.contains("fault summary"));
    assert!(summary.contains("reason=new reason"));
}

#[test]
fn service_rebuild_preserves_remote_candidate_cache_for_reconnect() {
    let mut service = RtcConnectionService::default();
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let session = XbxEngineSessionDto {
        session_id: "test-session".to_string(),
        target_type: XbxEngineTargetTypeDto::Cloud,
        turn_server: None,
    };

    service.rebuild(&session, &runtime_stats).unwrap();
    let duplicate = service
        .local_candidates_snapshot()
        .into_iter()
        .next()
        .expect("service local candidate");
    service
        .add_remote_ice_candidates(&[duplicate.clone(), duplicate], &runtime_stats)
        .unwrap();

    let state_before = service.state.lock().expect("connection state").clone();
    service.rebuild(&session, &runtime_stats).unwrap();
    let state_after = service.state.lock().expect("connection state");

    assert_eq!(
        state_after.remote_candidates.len(),
        state_before.remote_candidates.len(),
        "remote candidates should survive reconnect rebuild"
    );
    assert_eq!(
        state_after.pending_remote_candidates.len(),
        state_before.pending_remote_candidates.len(),
        "pending remote candidates should survive reconnect rebuild"
    );
    assert_eq!(
        state_after.remote_candidate_keys.len(),
        state_before.remote_candidate_keys.len(),
        "remote candidate keys should survive reconnect rebuild"
    );
    assert_eq!(
        state_after.pending_remote_candidate_keys.len(),
        state_before.pending_remote_candidate_keys.len(),
        "pending remote candidate keys should survive reconnect rebuild"
    );
}

#[test]
fn service_replays_pending_control_requests_after_control_close_and_rebuild() {
    let mut service = RtcConnectionService::default();
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let session = XbxEngineSessionDto {
        session_id: "test-session".to_string(),
        target_type: XbxEngineTargetTypeDto::Cloud,
        turn_server: None,
    };

    service.rebuild(&session, &runtime_stats).unwrap();
    let (
        mut answer_pc,
        mut answer_io,
        _message_dc_id,
        control_dc_id,
        _input_dc_id,
        _chat_dc_id,
        _saw_input_metadata,
        observed_payloads,
    ) = connect_service_to_answer_peer(&mut service, &runtime_stats);
    let control_dc_id = control_dc_id.expect("answer control channel id");
    assert!(
        service.control_service.is_control_ready(),
        "service should be control-ready before injecting failure"
    );
    assert!(
        observed_payloads
            .iter()
            .any(|(label, body)| label == MESSAGE_CHANNEL_LABEL && body.contains("Handshake")),
        "handshake should have been observed before failure injection"
    );

    {
        let mut answer_control_dc = answer_pc
            .data_channel(control_dc_id)
            .expect("answer control channel available");
        answer_control_dc.close().unwrap();
    }
    let close_deadline = Instant::now() + Duration::from_secs(3);
    let mut saw_control_closed = false;
    while Instant::now() < close_deadline {
        service.pump(&runtime_stats).unwrap();
        answer_io.pump(&mut answer_pc).unwrap();
        if runtime_stats.lock().ok().is_some_and(|stats| {
            stats.latest_observation_label.as_deref() == Some("rtcControlChannelClosed")
        }) {
            saw_control_closed = true;
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(saw_control_closed, "service should observe control close");

    assert!(service.request_video_keyframe(&runtime_stats).is_err());
    assert!(service.request_decoder_reset(&runtime_stats).is_err());
    assert!(
        service.control_service.has_pending_replay_actions(),
        "pending replay requests should be retained while reconnecting"
    );

    drop(answer_pc);
    drop(answer_io);
    service.rebuild(&session, &runtime_stats).unwrap();

    let (
        _reconnect_pc,
        _reconnect_io,
        reconnect_message_dc_id,
        reconnect_control_dc_id,
        reconnect_input_dc_id,
        reconnect_chat_dc_id,
        _saw_input_metadata,
        reconnect_payloads,
    ) = connect_service_to_answer_peer(&mut service, &runtime_stats);

    assert!(
        reconnect_message_dc_id.is_some(),
        "message channel should reopen"
    );
    assert!(
        reconnect_control_dc_id.is_some(),
        "control channel should reopen"
    );
    assert!(
        reconnect_input_dc_id.is_some(),
        "input channel should reopen"
    );
    assert!(reconnect_chat_dc_id.is_some(), "chat channel should reopen");
    assert!(
        service.control_service.is_control_ready(),
        "service should become control-ready again after reconnect"
    );

    let replay_keyframe = reconnect_payloads.iter().any(|(label, body)| {
        label == CONTROL_CHANNEL_LABEL && body.contains("\"message\":\"videoKeyframeRequested\"")
    });
    let replay_decoder_reset = reconnect_payloads.iter().any(|(label, body)| {
        label == CONTROL_CHANNEL_LABEL && body.contains("\"message\":\"decoderReset\"")
    });
    assert!(
        replay_keyframe,
        "reconnect should replay pending keyframe request"
    );
    assert!(
        replay_decoder_reset,
        "reconnect should replay pending decoder reset request"
    );
}

#[test]
fn service_records_handshake_and_control_ready_timestamps() {
    let mut service = RtcConnectionService::default();
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let session = XbxEngineSessionDto {
        session_id: "test-session".to_string(),
        target_type: XbxEngineTargetTypeDto::Cloud,
        turn_server: None,
    };

    service.rebuild(&session, &runtime_stats).unwrap();
    let (
        _answer_pc,
        _answer_io,
        _message_dc_id,
        _control_dc_id,
        _input_dc_id,
        _chat_dc_id,
        _saw_input_metadata,
        _observed_payloads,
    ) = connect_service_to_answer_peer(&mut service, &runtime_stats);

    let stats = runtime_stats.lock().expect("runtime stats");
    assert!(
        stats.message_handshake_acked_at_ms.is_some(),
        "handshake ack timestamp should be recorded"
    );
    assert!(
        stats.control_ready_at_ms.is_some(),
        "control ready timestamp should be recorded"
    );
    assert!(
        stats.control_ready_at_ms.unwrap() >= stats.message_handshake_acked_at_ms.unwrap(),
        "control ready should not precede handshake ack"
    );
}

#[derive(Debug)]
struct TestRtcPeerIo {
    socket: UdpSocket,
    local_addr: SocketAddr,
}

impl TestRtcPeerIo {
    fn bind() -> Result<Self, XbxEngineRuntimeError> {
        let socket = UdpSocket::bind("127.0.0.1:0").map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcTestIoBindFailed: {err}"))
        })?;
        socket.set_nonblocking(true).map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcTestIoSetNonblockingFailed: {err}"))
        })?;
        let local_addr = socket.local_addr().map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcTestIoLocalAddrFailed: {err}"))
        })?;
        Ok(Self { socket, local_addr })
    }

    fn local_candidate(&self) -> Result<RTCIceCandidateInit, XbxEngineRuntimeError> {
        let candidate = CandidateHostConfig {
            base_config: CandidateConfig {
                network: "udp".to_string(),
                address: self.local_addr.ip().to_string(),
                port: self.local_addr.port(),
                component: 1,
                ..Default::default()
            },
            ..Default::default()
        }
        .new_candidate_host()
        .map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcTestHostCandidateFailed: {err}"))
        })?;
        let mut candidate_init = RTCIceCandidate::from(&candidate).to_json().map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcTestCandidateToJsonFailed: {err}"))
        })?;
        candidate_init.sdp_mid = Some("0".to_string());
        candidate_init.sdp_mline_index = Some(0);
        Ok(candidate_init)
    }

    fn pump(
        &mut self,
        peer_connection: &mut rtc::peer_connection::RTCPeerConnection,
    ) -> Result<(), XbxEngineRuntimeError> {
        let mut buffer = [0u8; 2_048];

        for _ in 0..8 {
            let mut progressed = false;

            while let Some(deadline) = peer_connection.poll_timeout() {
                let now = Instant::now();
                if deadline > now {
                    break;
                }
                peer_connection.handle_timeout(now).map_err(|err| {
                    XbxEngineRuntimeError::new(format!(
                        "xbxEngineRtcTestHandleTimeoutFailed: {err}"
                    ))
                })?;
                progressed = true;
            }

            while let Some(message) = peer_connection.poll_write() {
                match self
                    .socket
                    .send_to(&message.message, message.transport.peer_addr)
                {
                    Ok(_) => progressed = true,
                    Err(err) if err.kind() == ErrorKind::WouldBlock => break,
                    Err(err) => {
                        return Err(XbxEngineRuntimeError::new(format!(
                            "xbxEngineRtcTestSocketSendFailed: {err}"
                        )));
                    }
                }
            }

            loop {
                match self.socket.recv_from(&mut buffer) {
                    Ok((size, peer_addr)) => {
                        peer_connection
                            .handle_read(TaggedBytesMut {
                                now: Instant::now(),
                                transport: TransportContext {
                                    local_addr: self.local_addr,
                                    peer_addr,
                                    transport_protocol: TransportProtocol::UDP,
                                    ecn: None,
                                },
                                message: BytesMut::from(&buffer[..size]),
                            })
                            .map_err(|err| {
                                XbxEngineRuntimeError::new(format!(
                                    "xbxEngineRtcTestHandleReadFailed: {err}"
                                ))
                            })?;
                        progressed = true;
                    }
                    Err(err) if err.kind() == ErrorKind::WouldBlock => break,
                    Err(err) => {
                        return Err(XbxEngineRuntimeError::new(format!(
                            "xbxEngineRtcTestSocketReadFailed: {err}"
                        )));
                    }
                }
            }

            if !progressed {
                break;
            }
        }

        Ok(())
    }
}

fn end_of_candidates_for_test() -> RTCIceCandidateInit {
    RTCIceCandidateInit {
        candidate: String::new(),
        sdp_mid: Some("0".to_string()),
        sdp_mline_index: Some(0),
        username_fragment: None,
        url: None,
    }
}

fn candidate_dto(
    candidate: &str,
    sdp_mid: Option<&str>,
    sdp_m_line_index: Option<u16>,
) -> XbxEngineIceCandidateDto {
    XbxEngineIceCandidateDto {
        candidate: candidate.to_string(),
        sdp_m_line_index,
        sdp_mid: sdp_mid.map(|value| value.to_string()),
    }
}

fn connect_service_to_answer_peer(
    service: &mut RtcConnectionService,
    runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
) -> (
    RTCPeerConnection,
    TestRtcPeerIo,
    Option<u16>,
    Option<u16>,
    Option<u16>,
    Option<u16>,
    bool,
    Vec<(String, String)>,
) {
    let offer = service
        .create_raw_offer(&Default::default(), runtime_stats)
        .unwrap();
    let service_candidates = service.local_candidates_snapshot();
    assert!(
        !service_candidates.is_empty(),
        "service local candidates should not be empty"
    );

    let mut answer_pc = RTCPeerConnectionBuilder::new()
        .with_configuration(RTCConfigurationBuilder::new().build())
        .build()
        .unwrap();
    let mut answer_io = TestRtcPeerIo::bind().unwrap();
    let answer_candidate = answer_io.local_candidate().unwrap();

    answer_pc
        .set_remote_description(RTCSessionDescription::offer(offer).unwrap())
        .unwrap();
    answer_pc
        .add_local_candidate(answer_candidate.clone())
        .unwrap();
    answer_pc
        .add_local_candidate(end_of_candidates_for_test())
        .unwrap();
    for candidate in &service_candidates {
        answer_pc
            .add_remote_candidate(dto_to_rtc_candidate(candidate))
            .unwrap();
    }
    let answer = answer_pc.create_answer(None).unwrap();
    answer_pc.set_local_description(answer.clone()).unwrap();

    service
        .apply_remote_description(
            &answer.sdp,
            &[XbxEngineIceCandidateDto {
                candidate: answer_candidate.candidate.clone(),
                sdp_m_line_index: answer_candidate.sdp_mline_index,
                sdp_mid: answer_candidate.sdp_mid.clone(),
            }],
            runtime_stats,
        )
        .unwrap();

    let mut answer_connected = false;
    let mut answer_message_dc_id = None;
    let mut answer_control_dc_id = None;
    let mut answer_input_dc_id = None;
    let mut answer_chat_dc_id = None;
    let mut observed_payloads = Vec::new();
    let mut handshake_ack_sent = false;
    let mut saw_input_metadata = false;
    let mut ready_streak: u8 = 0;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        service.pump(runtime_stats).unwrap();
        answer_io.pump(&mut answer_pc).unwrap();
        while let Some(event) = answer_pc.poll_event() {
            match event {
                RTCPeerConnectionEvent::OnConnectionStateChangeEvent(
                    RTCPeerConnectionState::Connected,
                ) => {
                    answer_connected = true;
                }
                RTCPeerConnectionEvent::OnConnectionStateChangeEvent(
                    RTCPeerConnectionState::Failed,
                ) => panic!("answer peer connection failed"),
                RTCPeerConnectionEvent::OnDataChannel(RTCDataChannelEvent::OnOpen(dc_id)) => {
                    let label = answer_pc
                        .data_channel(dc_id)
                        .expect("answer data channel")
                        .label()
                        .to_string();
                    match label.as_str() {
                        MESSAGE_CHANNEL_LABEL => {
                            let _ = answer_message_dc_id.get_or_insert(dc_id);
                        }
                        CONTROL_CHANNEL_LABEL => {
                            let _ = answer_control_dc_id.get_or_insert(dc_id);
                        }
                        INPUT_CHANNEL_LABEL => {
                            let _ = answer_input_dc_id.get_or_insert(dc_id);
                        }
                        CHAT_CHANNEL_LABEL => {
                            let _ = answer_chat_dc_id.get_or_insert(dc_id);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        while let Some(message) = answer_pc.poll_read() {
            if let rtc::peer_connection::message::RTCMessage::DataChannelMessage(
                channel_id,
                payload,
            ) = message
            {
                let label = answer_pc
                    .data_channel(channel_id)
                    .expect("answer data channel")
                    .label()
                    .to_string();
                let body = String::from_utf8_lossy(payload.data.as_ref()).to_string();
                observed_payloads.push((label.clone(), body.clone()));
                if label == INPUT_CHANNEL_LABEL && !payload.is_string {
                    let bytes = payload.data.as_ref();
                    saw_input_metadata =
                        bytes.len() == 15 && u16::from_le_bytes([bytes[0], bytes[1]]) == 8;
                }
                if label == MESSAGE_CHANNEL_LABEL && body.contains("\"type\":\"Handshake\"") {
                    let mut answer_dc = answer_pc
                        .data_channel(channel_id)
                        .expect("answer message channel");
                    answer_dc
                        .send_text(HANDSHAKE_ACK_PAYLOAD.to_string())
                        .unwrap();
                    handshake_ack_sent = true;
                }
            }
        }

        if handshake_ack_sent {
            answer_io.pump(&mut answer_pc).unwrap();
            service.pump(runtime_stats).unwrap();
            answer_io.pump(&mut answer_pc).unwrap();
            handshake_ack_sent = false;
        }

        let ready = answer_connected
            && service.control_service.is_control_ready()
            && answer_message_dc_id.is_some()
            && answer_control_dc_id.is_some()
            && answer_input_dc_id.is_some()
            && answer_chat_dc_id.is_some();
        if ready {
            ready_streak = ready_streak.saturating_add(1);
        } else {
            ready_streak = 0;
        }
        if ready_streak >= 2 {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    assert!(answer_connected, "answer peer should reach connected");
    assert!(
        answer_message_dc_id.is_some(),
        "message channel should open"
    );
    assert!(
        answer_control_dc_id.is_some(),
        "control channel should open"
    );
    assert!(answer_input_dc_id.is_some(), "input channel should open");
    assert!(answer_chat_dc_id.is_some(), "chat channel should open");
    let control_state = service.control_service.state().clone();
    let latest_observation_label = runtime_stats
        .lock()
        .ok()
        .and_then(|stats| stats.latest_observation_label.clone());
    assert!(
        service.control_service.is_control_ready(),
        "service control channel should become ready, observed payloads: {observed_payloads:?}, state: message_open={} message_acked={} post_handshake_sent={} control_open={} control_started={} control_bootstrapped_after_handshake={} pending_keyframe={} pending_decoder_reset={} lifecycle={:?} latest_observation={latest_observation_label:?}",
        control_state.message_channel_open,
        control_state.message_handshake_acked,
        control_state.post_handshake_messages_sent,
        control_state.control_channel_open,
        control_state.control_started,
        control_state.control_bootstrapped_after_handshake,
        control_state.pending_keyframe_request,
        control_state.pending_decoder_reset,
        service.lifecycle_state
    );

    let replay_deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < replay_deadline {
        service.pump(runtime_stats).unwrap();
        answer_io.pump(&mut answer_pc).unwrap();
        while let Some(message) = answer_pc.poll_read() {
            if let rtc::peer_connection::message::RTCMessage::DataChannelMessage(
                channel_id,
                payload,
            ) = message
            {
                let label = answer_pc
                    .data_channel(channel_id)
                    .expect("answer data channel")
                    .label()
                    .to_string();
                let body = String::from_utf8_lossy(payload.data.as_ref()).to_string();
                observed_payloads.push((label, body));
            }
        }
        let replay_keyframe = observed_payloads.iter().any(|(label, body)| {
            label == CONTROL_CHANNEL_LABEL
                && body.contains("\"message\":\"videoKeyframeRequested\"")
        });
        let replay_decoder_reset = observed_payloads.iter().any(|(label, body)| {
            label == CONTROL_CHANNEL_LABEL && body.contains("\"message\":\"decoderReset\"")
        });
        if replay_keyframe && replay_decoder_reset {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    (
        answer_pc,
        answer_io,
        answer_message_dc_id,
        answer_control_dc_id,
        answer_input_dc_id,
        answer_chat_dc_id,
        saw_input_metadata,
        observed_payloads,
    )
}
