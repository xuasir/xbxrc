use rtc::interceptor::{
    NackGeneratorBuilder, NackGeneratorInterceptor, NackResponderBuilder, NackResponderInterceptor,
    NoopInterceptor, ReceiverReportBuilder, ReceiverReportInterceptor, Registry,
    SenderReportBuilder, SenderReportInterceptor,
};
use rtc::peer_connection::configuration::interceptor_registry::configure_simulcast_extension_headers;
use rtc::peer_connection::configuration::media_engine::{
    MediaEngine, MIME_TYPE_H264, MIME_TYPE_OPUS,
};
use rtc::peer_connection::configuration::{RTCConfigurationBuilder, RTCIceServer};
use rtc::peer_connection::RTCPeerConnection;
use rtc::peer_connection::RTCPeerConnectionBuilder;
use rtc::rtp_transceiver::rtp_sender::{
    RTCPFeedback, RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters,
    RTCRtpEncodingParameters, RTCRtpHeaderExtensionCapability, RtpCodecKind,
    TYPE_RTCP_FB_GOOG_REMB, TYPE_RTCP_FB_NACK,
};
use rtc::rtp_transceiver::{RTCRtpTransceiverDirection, RTCRtpTransceiverInit};
use rtc::shared::error::Result as SharedResult;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::api::runtime::XbxEngineWebRtcRuntimeConfig;
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::stats::now_ms_f64;
use crate::{XbxEngineMediaRuntimeStats, XbxEngineRtcBuilderObservation, XbxEngineRuntimeError};
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

static RTC_BUILDER_OBSERVATION_ID: AtomicU64 = AtomicU64::new(0);

pub(super) type ControlledTwccInterceptor = SenderReportInterceptor<
    ReceiverReportInterceptor<NackResponderInterceptor<NackGeneratorInterceptor<NoopInterceptor>>>,
>;

pub(super) type ControlledPeerConnection = RTCPeerConnection<ControlledTwccInterceptor>;

pub(super) fn build_ice_servers(session: &XbxEngineSessionDto) -> Vec<RTCIceServer> {
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
    ice_servers
}

pub(super) fn build_peer_connection(
    session: &XbxEngineSessionDto,
    runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    runtime_config: &XbxEngineWebRtcRuntimeConfig,
) -> Result<ControlledPeerConnection, XbxEngineRuntimeError> {
    let mut media_engine = MediaEngine::default();
    media_engine.register_default_codecs().map_err(|err| {
        XbxEngineRuntimeError::new(format!("xbxEngineRtcRegisterDefaultCodecsFailed: {err}"))
    })?;
    // 对齐当前 RTC 主线：在默认 codec 之外补齐我们稳定依赖的 H264 family。
    register_owned_h264_codecs(&mut media_engine)?;
    let interceptor_registry = build_controlled_twcc_registry(
        &mut media_engine,
        runtime_stats.clone(),
        runtime_config.video_pipeline.feedback_interval_ms,
    )?;
    RuntimeStatsSink::new(runtime_stats.clone()).record_rtc_builder_observation(
        XbxEngineRtcBuilderObservation {
            observation_id: RTC_BUILDER_OBSERVATION_ID.fetch_add(1, Ordering::Relaxed) + 1,
            controlled_twcc_registry: true,
            feedback_interval_ms: runtime_config.video_pipeline.feedback_interval_ms as f64,
            registered_header_extensions: vec![
                "video:http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01"
                    .to_string(),
                "audio:http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01"
                    .to_string(),
            ],
            registered_rtcp_feedback: vec![
                "video:nack".to_string(),
                "video:nack:pli".to_string(),
                "video:goog-remb".to_string(),
                "video:transport-cc".to_string(),
                "audio:goog-remb".to_string(),
                "audio:transport-cc".to_string(),
            ],
            observed_at_ms: now_ms_f64(),
        },
    );

    let ice_servers = build_ice_servers(session);
    let ice_server_urls = ice_servers
        .iter()
        .flat_map(|server| server.urls.iter().cloned())
        .collect::<Vec<_>>();
    crate::xbx_log_debug!(
        "[xbxengine][rtc-builder] build peer connection session={} target={:?} turn_configured={} ice_server_count={} ice_server_urls={:?}",
        session.session_id,
        session.target_type,
        session.turn_server.is_some(),
        ice_server_urls.len(),
        ice_server_urls,
    );
    let configuration = RTCConfigurationBuilder::new().with_ice_servers(ice_servers);
    RTCPeerConnectionBuilder::new()
        .with_configuration(configuration.build())
        .with_media_engine(media_engine)
        .with_interceptor_registry(interceptor_registry)
        .build()
        .map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcBuildPeerConnectionFailed: {err}"))
        })
}

fn build_controlled_twcc_registry(
    media_engine: &mut MediaEngine,
    _runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    _feedback_interval_ms: u64,
) -> Result<Registry<ControlledTwccInterceptor>, XbxEngineRuntimeError> {
    // 显式组装 interceptor registry，避免继续依赖隐式默认链路。
    let registry = Registry::new();
    configure_nack_feedback_support(media_engine);
    configure_simulcast_extension_headers(media_engine).map_err(|err| {
        XbxEngineRuntimeError::new(format!(
            "xbxEngineRtcConfigureSimulcastHeadersFailed: {err}"
        ))
    })?;
    configure_twcc_receiver_feedback_support(media_engine).map_err(|err| {
        XbxEngineRuntimeError::new(format!("xbxEngineRtcConfigureTwccFailed: {err}"))
    })?;
    Ok(registry
        .with(NackGeneratorBuilder::new().build())
        .with(NackResponderBuilder::new().build())
        .with(ReceiverReportBuilder::new().build())
        .with(SenderReportBuilder::new().build()))
}

fn configure_nack_feedback_support(media_engine: &mut MediaEngine) {
    media_engine.register_feedback(
        RTCPFeedback {
            typ: TYPE_RTCP_FB_NACK.to_string(),
            parameter: String::new(),
        },
        RtpCodecKind::Video,
    );
    media_engine.register_feedback(
        RTCPFeedback {
            typ: TYPE_RTCP_FB_NACK.to_string(),
            parameter: "pli".to_string(),
        },
        RtpCodecKind::Video,
    );
}

fn configure_twcc_receiver_feedback_support(media_engine: &mut MediaEngine) -> SharedResult<()> {
    const TRANSPORT_CC_URI: &str =
        "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01";
    media_engine.register_feedback(
        RTCPFeedback {
            typ: "transport-cc".to_string(),
            ..Default::default()
        },
        RtpCodecKind::Video,
    );
    media_engine.register_feedback(
        RTCPFeedback {
            typ: TYPE_RTCP_FB_GOOG_REMB.to_string(),
            ..Default::default()
        },
        RtpCodecKind::Video,
    );
    media_engine.register_header_extension(
        RTCRtpHeaderExtensionCapability {
            uri: TRANSPORT_CC_URI.to_string(),
        },
        RtpCodecKind::Video,
        None,
    )?;
    media_engine.register_feedback(
        RTCPFeedback {
            typ: "transport-cc".to_string(),
            ..Default::default()
        },
        RtpCodecKind::Audio,
    );
    media_engine.register_feedback(
        RTCPFeedback {
            typ: TYPE_RTCP_FB_GOOG_REMB.to_string(),
            ..Default::default()
        },
        RtpCodecKind::Audio,
    );
    media_engine.register_header_extension(
        RTCRtpHeaderExtensionCapability {
            uri: TRANSPORT_CC_URI.to_string(),
        },
        RtpCodecKind::Audio,
        None,
    )?;
    Ok(())
}

pub(super) fn configure_offer_primitives(
    peer_connection: &mut ControlledPeerConnection,
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
            typ: TYPE_RTCP_FB_GOOG_REMB.to_string(),
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
    // 对齐 Xbox 云端兼容口径：Main(4d) 优先，其次 42e / 420；
    // 64 family 保留，但不作为首选 family。
    vec![
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
