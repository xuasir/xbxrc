use rtc::peer_connection::configuration::media_engine::{
    MediaEngine, MIME_TYPE_H264, MIME_TYPE_OPUS,
};
use rtc::peer_connection::configuration::{RTCConfigurationBuilder, RTCIceServer};
use rtc::peer_connection::RTCPeerConnection;
use rtc::peer_connection::RTCPeerConnectionBuilder;
use rtc::rtp_transceiver::rtp_sender::{
    RTCPFeedback, RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters,
    RTCRtpEncodingParameters, RtpCodecKind,
};
use rtc::rtp_transceiver::{RTCRtpTransceiverDirection, RTCRtpTransceiverInit};

use crate::transport::rtc::stats::now_ms_f64;
use crate::XbxEngineRuntimeError;
use xbxengine_protocol::XbxEngineSessionDto;

const DEFAULT_ICE_SERVERS: [&str; 7] = [
    "stun:worldaz.relay.teams.microsoft.com:3478",
    "stun:stun.l.google.com:19302",
    "stun:stun1.l.google.com:19302",
    "stun:relay1.expressturn.com",
    "stun:relay2.expressturn.com",
    "stun:stun.kinesisvideo.us-east-1.amazonaws.com:443",
    "stun:stun.douyucdn.cn:18000",
];

pub(super) fn build_peer_connection(
    session: &XbxEngineSessionDto,
) -> Result<RTCPeerConnection, XbxEngineRuntimeError> {
    let mut media_engine = MediaEngine::default();
    media_engine.register_default_codecs().map_err(|err| {
        XbxEngineRuntimeError::new(format!("xbxEngineRtcRegisterDefaultCodecsFailed: {err}"))
    })?;
    // 对齐旧 webrtc 主线：在默认 codec 之外补齐我们稳定依赖的 H264 family。
    register_owned_h264_codecs(&mut media_engine)?;

    let mut ice_servers = Vec::new();
    if !cfg!(test) {
        ice_servers.push(RTCIceServer {
            urls: DEFAULT_ICE_SERVERS
                .iter()
                .map(|url| (*url).to_string())
                .collect(),
            ..Default::default()
        });
    }
    if let Some(turn_server) = session.turn_server.as_ref() {
        ice_servers.push(RTCIceServer {
            urls: vec![turn_server.url.clone()],
            username: turn_server.username.clone(),
            credential: turn_server.credential.clone(),
        });
    }
    let configuration = RTCConfigurationBuilder::new().with_ice_servers(ice_servers);
    RTCPeerConnectionBuilder::new()
        .with_configuration(configuration.build())
        .with_media_engine(media_engine)
        .build()
        .map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcBuildPeerConnectionFailed: {err}"))
        })
}

pub(super) fn configure_offer_primitives(
    peer_connection: &mut RTCPeerConnection,
) -> Result<(), XbxEngineRuntimeError> {
    // 对齐旧 transport 的 offer 结构：audio + video + application 三段必须同时出现。
    peer_connection
        .add_transceiver_from_kind(
            RtpCodecKind::Audio,
            Some(RTCRtpTransceiverInit {
                direction: RTCRtpTransceiverDirection::Sendrecv,
                streams: vec![],
                send_encodings: vec![RTCRtpEncodingParameters {
                    // rtc 对 sendrecv 要求显式 base encoding，且 codec 必须能在 MediaEngine 命中。
                    rtp_coding_parameters: RTCRtpCodingParameters {
                        ssrc: Some(generate_offer_audio_ssrc()),
                        ..Default::default()
                    },
                    codec: RTCRtpCodec {
                        mime_type: MIME_TYPE_OPUS.to_string(),
                        clock_rate: 48_000,
                        channels: 2,
                        sdp_fmtp_line: "minptime=10;useinbandfec=1".to_string(),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
            }),
        )
        .map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcAddAudioTransceiverFailed: {err}"))
        })?;

    // rtc 0.9.0 没有公开的 transceiver.set_codec_preferences；
    // phase1 里通过 MediaEngine 预注册 + 上层 SDP policy 重排来显式约束视频偏好。
    peer_connection
        .add_transceiver_from_kind(
            RtpCodecKind::Video,
            Some(RTCRtpTransceiverInit {
                direction: RTCRtpTransceiverDirection::Recvonly,
                streams: vec![],
                send_encodings: vec![],
            }),
        )
        .map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcAddVideoTransceiverFailed: {err}"))
        })?;

    Ok(())
}

pub(super) fn generate_offer_audio_ssrc() -> u32 {
    let seed = now_ms_f64() as u32;
    if seed == 0 {
        1
    } else {
        seed
    }
}

pub(super) fn register_owned_h264_codecs(
    media_engine: &mut MediaEngine,
) -> Result<(), XbxEngineRuntimeError> {
    for codec in build_owned_h264_codec_preferences() {
        media_engine
            .register_codec(codec, RtpCodecKind::Video)
            .map_err(|err| {
                XbxEngineRuntimeError::new(format!(
                    "xbxEngineRtcRegisterOwnedH264CodecFailed: {err}"
                ))
            })?;
    }
    Ok(())
}

pub(super) fn build_owned_h264_codec_preferences() -> Vec<RTCRtpCodecParameters> {
    let video_rtcp_feedback = vec![
        RTCPFeedback {
            typ: "goog-remb".to_string(),
            parameter: String::new(),
        },
        RTCPFeedback {
            typ: "transport-cc".to_string(),
            parameter: String::new(),
        },
        RTCPFeedback {
            typ: "ccm".to_string(),
            parameter: "fir".to_string(),
        },
        RTCPFeedback {
            typ: "nack".to_string(),
            parameter: String::new(),
        },
        RTCPFeedback {
            typ: "nack".to_string(),
            parameter: "pli".to_string(),
        },
    ];
    // 与旧主线一致：高 -> 主 -> 受限基线 -> 基线，最后附加 RTX(apt=124)。
    vec![
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: MIME_TYPE_H264.to_string(),
                clock_rate: 90_000,
                channels: 0,
                sdp_fmtp_line:
                    "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=640032"
                        .to_string(),
                rtcp_feedback: video_rtcp_feedback.clone(),
            },
            payload_type: 123,
        },
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: MIME_TYPE_H264.to_string(),
                clock_rate: 90_000,
                channels: 0,
                sdp_fmtp_line:
                    "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=4d0032"
                        .to_string(),
                rtcp_feedback: video_rtcp_feedback.clone(),
            },
            payload_type: 124,
        },
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: MIME_TYPE_H264.to_string(),
                clock_rate: 90_000,
                channels: 0,
                sdp_fmtp_line:
                    "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"
                        .to_string(),
                rtcp_feedback: video_rtcp_feedback.clone(),
            },
            payload_type: 125,
        },
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: MIME_TYPE_H264.to_string(),
                clock_rate: 90_000,
                channels: 0,
                sdp_fmtp_line:
                    "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42001f"
                        .to_string(),
                rtcp_feedback: video_rtcp_feedback,
            },
            payload_type: 102,
        },
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: "video/rtx".to_string(),
                clock_rate: 90_000,
                channels: 0,
                sdp_fmtp_line: "apt=124".to_string(),
                rtcp_feedback: vec![],
            },
            payload_type: 97,
        },
    ]
}
