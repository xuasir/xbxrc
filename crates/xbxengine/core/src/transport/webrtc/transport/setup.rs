use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, AtomicU64};
use std::sync::{Arc, Mutex};

use tokio::runtime::Handle;
use tokio::sync::mpsc;
use webrtc::{
    api::media_engine::MIME_TYPE_H264,
    data_channel::{data_channel_init::RTCDataChannelInit, RTCDataChannel},
    ice_transport::{ice_candidate::RTCIceCandidate, ice_server::RTCIceServer},
    peer_connection::{
        configuration::RTCConfiguration, peer_connection_state::RTCPeerConnectionState,
        RTCPeerConnection,
    },
    rtp_transceiver::{
        rtp_codec::{
            RTCRtpCodecCapability, RTCRtpCodecParameters, RTCRtpHeaderExtensionCapability,
            RTPCodecType,
        },
        rtp_transceiver_direction::RTCRtpTransceiverDirection,
        RTCPFeedback, RTCRtpTransceiverInit, TYPE_RTCP_FB_CCM, TYPE_RTCP_FB_GOOG_REMB,
        TYPE_RTCP_FB_NACK, TYPE_RTCP_FB_TRANSPORT_CC,
    },
};

use crate::{
    media::video::render::renderer::XbxRenderState,
    runtime_stats_sink::RuntimeStatsSink,
    transport::adapter::{FrameSource, TransportFeedbackPort, WebrtcVideoAdapter},
    transport::webrtc::audio_output::XbxRemoteAudioPlaybackSession,
    transport::webrtc::data_channel::{install_data_channel_contracts, XbxDataChannelState},
    transport::webrtc::nack_scheduler::NackSchedulerConfig,
    transport::webrtc::twcc_owned_receiver::OwnedTwccReceiverBuilder,
    XbxEngineMediaRuntimeStats, XbxEngineRuntimeError, XbxEngineVideoTrackStatus,
    XbxEngineWebRtcRuntimeConfig,
};
use xbxengine_protocol::{
    XbxEngineIceCandidateDto, XbxEngineTransportStateDto, XbxEngineTurnServerDto,
};

use super::video_track_stats::spawn_video_track_stats_loop;

const DEFAULT_ICE_SERVERS: [&str; 7] = [
    "stun:worldaz.relay.teams.microsoft.com:3478",
    "stun:stun.l.google.com:19302",
    "stun:stun1.l.google.com:19302",
    "stun:relay1.expressturn.com",
    "stun:relay2.expressturn.com",
    "stun:stun.kinesisvideo.us-east-1.amazonaws.com:443",
    "stun:stun.douyucdn.cn:18000",
];
const TRANSPORT_CC_URI: &str =
    "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01";

struct PeerConnectionFeedbackPort {
    peer_connection: Arc<RTCPeerConnection>,
}

impl PeerConnectionFeedbackPort {
    fn new(peer_connection: Arc<RTCPeerConnection>) -> Self {
        Self { peer_connection }
    }
}

impl TransportFeedbackPort for PeerConnectionFeedbackPort {
    fn send_transport_layer_nack<'a>(
        &'a self,
        media_ssrc: u32,
        sequences: &'a [u16],
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>> {
        use webrtc::rtcp::transport_feedbacks::transport_layer_nack::{
            nack_pairs_from_sequence_numbers, TransportLayerNack,
        };

        // feedback port 只负责发 RTCP，不向 adapter 暴露 PeerConnection 实体。
        let nack = TransportLayerNack {
            sender_ssrc: 0,
            media_ssrc,
            nacks: nack_pairs_from_sequence_numbers(sequences),
        };
        let peer_connection = self.peer_connection.clone();
        Box::pin(async move {
            peer_connection
                .write_rtcp(&[Box::new(nack)])
                .await
                .map(|_| ())
                .map_err(|error| error.to_string())
        })
    }
}

// peer connection 装配与 callback 安装收在一个粗模块里，避免继续膨胀 core.rs。
pub(crate) fn install_peer_connection_callbacks(
    peer_connection: &Arc<RTCPeerConnection>,
    local_candidates: Arc<Mutex<Vec<XbxEngineIceCandidateDto>>>,
    local_ice_gathering_complete: Arc<Mutex<bool>>,
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    _render_state: Arc<Mutex<XbxRenderState>>,
    webrtc_config: XbxEngineWebRtcRuntimeConfig,
    frame_source_tx: Arc<Mutex<Option<mpsc::Sender<Box<dyn FrameSource>>>>>,
    audio_playback_session: Arc<Mutex<Option<XbxRemoteAudioPlaybackSession>>>,
    audio_volume_bits: Arc<AtomicU32>,
    task_generation: Arc<AtomicU64>,
    current_generation: u64,
) {
    let runtime_stats_sink = RuntimeStatsSink::new(runtime_stats.clone());

    peer_connection.on_ice_candidate(Box::new(move |candidate: Option<RTCIceCandidate>| {
        let local_candidates = local_candidates.clone();
        let local_ice_gathering_complete = local_ice_gathering_complete.clone();
        Box::pin(async move {
            let Some(candidate) = candidate else {
                if let Ok(mut complete) = local_ice_gathering_complete.lock() {
                    *complete = true;
                }
                crate::xbx_log_debug!("[xbxengine][webrtc-rs] local ice gathering complete");
                return;
            };
            match candidate.to_json() {
                Ok(json) => {
                    if let Ok(mut list) = local_candidates.lock() {
                        list.push(XbxEngineIceCandidateDto {
                            candidate: json.candidate,
                            sdp_m_line_index: json.sdp_mline_index,
                            sdp_mid: json.sdp_mid,
                        });
                        crate::xbx_log_debug!(
                            "[xbxengine][webrtc-rs] local ice candidate gathered mline={} total={}",
                            list.last()
                                .and_then(|value| value.sdp_m_line_index)
                                .unwrap_or_default(),
                            list.len()
                        );
                    }
                }
                Err(error) => {
                    crate::xbx_log_debug!(
                        "[xbxengine][webrtc-rs] local ice candidate serialize failed: {error}"
                    );
                }
            }
        })
    }));

    let runtime_stats_for_state = runtime_stats.clone();
    peer_connection.on_peer_connection_state_change(Box::new(move |state| {
        let runtime_stats = runtime_stats_for_state.clone();
        Box::pin(async move {
            RuntimeStatsSink::new(runtime_stats).update(|stats| {
                stats.transport_state = map_peer_connection_state(state);
            });
            crate::xbx_log_info!("[xbxengine][webrtc-rs] peer connection state={state}");
        })
    }));

    peer_connection.on_data_channel(Box::new(|channel| {
        Box::pin(async move {
            crate::xbx_log_debug!(
                "[xbxengine][webrtc-rs] remote data channel label={} protocol={} state={:?}",
                channel.label(),
                channel.protocol(),
                channel.ready_state()
            );
        })
    }));

    let runtime_stats_for_track = runtime_stats.clone();
    let peer_connection_for_track = peer_connection.clone();
    let webrtc_config_for_track = webrtc_config.clone();
    let audio_playback_session_for_track = audio_playback_session.clone();
    let audio_volume_bits_for_track = audio_volume_bits.clone();

    peer_connection.on_track(Box::new(move |track, _, transceiver| {
        let frame_source_tx = frame_source_tx.clone();
        let pc_captured = peer_connection_for_track.clone();
        let config_captured = webrtc_config_for_track.clone();
        let runtime_stats_captured = runtime_stats_for_track.clone();
        let runtime_stats_sink = runtime_stats_sink.clone();
        let task_generation_for_track = task_generation.clone();
        let audio_playback_session = audio_playback_session_for_track.clone();
        let audio_volume_bits = audio_volume_bits_for_track.clone();

        Box::pin(async move {
            crate::xbx_log_info!(
                "[xbxengine][webrtc-rs] ON_TRACK received: kind={} ssrc={} mime={} transceiver_kind={} mid={}",
                track.kind(),
                track.ssrc(),
                track.codec().capability.mime_type,
                transceiver.kind(),
                transceiver.mid().as_deref().unwrap_or("")
            );
            crate::xbx_log_debug!(
                "[xbxengine][webrtc-rs] remote track kind={} mime={} transceiver_kind={} mid={}",
                track.kind(),
                track.codec().capability.mime_type,
                transceiver.kind(),
                transceiver.mid().as_deref().unwrap_or("")
            );

            let track_kind = track.kind();
            let transceiver_kind = Some(transceiver.kind());
            let is_audio = track_kind == webrtc::rtp_transceiver::rtp_codec::RTPCodecType::Audio;
            let video_mime_type = normalize_remote_track_mime(&track.codec().capability.mime_type);
            let is_primary_video_track =
                is_primary_remote_video_track(track_kind, transceiver_kind, video_mime_type.as_deref());

            if is_primary_video_track {
                if video_mime_type.as_deref() != Some("video/h264") {
                    crate::xbx_log_warn!(
                        "[xbxengine][webrtc-rs] mounting video track via compatibility path kind={} transceiver_kind={} mime={}",
                        track_kind,
                        transceiver.kind(),
                        video_mime_type.as_deref().unwrap_or("")
                    );
                }
                let observed_at_ms = now_ms_f64();
                update_video_track_status(
                    &runtime_stats_sink,
                    XbxEngineVideoTrackStatus {
                        state: "remoteTrackAttached".to_string(),
                        video_width: None,
                        video_height: None,
                        mime_type: video_mime_type.clone(),
                        transport_state: XbxEngineTransportStateDto::Connected,
                        video_bytes_total: 0,
                        video_packet_count_total: 0,
                        audio_bytes_total: runtime_stats_captured
                            .lock()
                            .ok()
                            .map(|shared| shared.inbound_audio_bytes_total)
                            .unwrap_or(0),
                        observed_at_ms,
                    },
                );
                let jitter_buffer_size = config_captured.video_pipeline.jitter_buffer_max_packets;
                let idle_timeout = std::time::Duration::from_millis(
                    config_captured.video_pipeline.idle_timeout_ms,
                );

                crate::xbx_log_info!(
                    "[xbxengine][webrtc-rs] mounting video track with jitter_buffer={} idle_timeout={:?}",
                    jitter_buffer_size,
                    idle_timeout
                );

                let feedback_port: Arc<dyn TransportFeedbackPort> =
                    Arc::new(PeerConnectionFeedbackPort::new(pc_captured.clone()));
                let adapter = WebrtcVideoAdapter::new(
                    track.clone(),
                    feedback_port,
                    runtime_stats_captured.clone(),
                    jitter_buffer_size,
                    std::time::Duration::from_millis(
                        config_captured.video_pipeline.jitter_buffer_min_delay_ms,
                    ),
                    std::time::Duration::from_millis(
                        config_captured.video_pipeline.jitter_buffer_max_delay_ms,
                    ),
                    idle_timeout,
                    NackSchedulerConfig {
                        max_age_ms: config_captured.video_pipeline.nack_max_age_ms,
                        frame_deadline_ms: config_captured
                            .video_pipeline
                            .late_frame_drop_threshold_ms,
                        burst_count: config_captured.video_pipeline.nack_burst_count,
                        retry_interval_ms: config_captured.video_pipeline.nack_retry_interval_ms,
                        max_retry_count: config_captured.video_pipeline.nack_max_retry_count,
                    },
                );
                let source: Box<dyn FrameSource> = Box::new(adapter);
                if let Ok(guard) = frame_source_tx.lock() {
                    if let Some(tx) = guard.as_ref() {
                        if let Err(e) = tx.try_send(source) {
                            crate::xbx_log_error!(
                                "[xbxengine][webrtc-rs] Failed to mount new video track: {}",
                                e
                            );
                        }
                    } else {
                        crate::xbx_log_error!(
                            "[xbxengine][webrtc-rs] frame_source_tx is None! Supervisor task is dead?"
                        );
                    }
                }

                spawn_video_track_stats_loop(
                    track.clone(),
                    pc_captured.clone(),
                    runtime_stats_captured.clone(),
                    config_captured.clone(),
                    task_generation_for_track,
                    current_generation,
                    video_mime_type.clone(),
                );
            } else if is_audio {
                crate::xbx_log_info!(
                    "[xbxengine][webrtc-rs] mounting audio playback track mime={}",
                    track.codec().capability.mime_type
                );
                mount_remote_audio_track(
                    track.clone(),
                    runtime_stats_captured.clone(),
                    audio_playback_session,
                    audio_volume_bits,
                );
            } else {
                tokio::spawn(async move {
                    while let Ok(_) = track.read_rtp().await {}
                });
            }
        })
    }));
}

fn mount_remote_audio_track(
    track: Arc<webrtc::track::track_remote::TrackRemote>,
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    audio_playback_session: Arc<Mutex<Option<XbxRemoteAudioPlaybackSession>>>,
    audio_volume_bits: Arc<AtomicU32>,
) {
    match XbxRemoteAudioPlaybackSession::start(
        track.clone(),
        runtime_stats.clone(),
        audio_volume_bits,
    ) {
        Ok(session) => {
            if let Ok(mut current_session) = audio_playback_session.lock() {
                if let Some(previous) = current_session.replace(session) {
                    previous.stop();
                }
            } else {
                session.stop();
            }
        }
        Err(error) => {
            crate::xbx_log_error!(
                "[xbxengine][webrtc-rs][audio] remote playback unavailable, fallback to drain: {error}"
            );
            spawn_audio_drain_task(track, runtime_stats);
        }
    }
}

fn spawn_audio_drain_task(
    track: Arc<webrtc::track::track_remote::TrackRemote>,
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
) {
    let runtime_stats = RuntimeStatsSink::new(runtime_stats);
    tokio::spawn(async move {
        let mut total_audio_bytes = 0u64;
        let mut last_audio_sample_bytes = 0u64;
        let mut last_audio_sample_at_ms = now_ms_f64();
        while let Ok((rtp, _)) = track.read_rtp().await {
            total_audio_bytes = total_audio_bytes.saturating_add(rtp.payload.len() as u64);
            let now_ms = now_ms_f64();
            let elapsed_ms = (now_ms - last_audio_sample_at_ms).max(0.0);
            runtime_stats.update(|shared| {
                shared.inbound_audio_bytes_total = total_audio_bytes;
                if elapsed_ms >= 250.0 {
                    let delta_bytes = total_audio_bytes.saturating_sub(last_audio_sample_bytes);
                    let audio_kbps = (delta_bytes * 8) as f64 / elapsed_ms.max(1.0);
                    shared.inbound_audio_bitrate_kbps = Some(audio_kbps.max(0.0));
                    shared.inbound_bitrate_kbps = Some(
                        shared.inbound_video_bitrate_kbps.unwrap_or(0.0) + audio_kbps.max(0.0),
                    );
                    last_audio_sample_bytes = total_audio_bytes;
                    last_audio_sample_at_ms = now_ms;
                }
                shared.inbound_bytes_total =
                    shared.inbound_video_bytes_total + shared.inbound_audio_bytes_total;
                if shared.latest_video_track_status.is_none()
                    && shared.inbound_audio_bytes_total > 0
                    && shared.inbound_video_bytes_total == 0
                {
                    shared.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
                        state: "audioOnly".to_string(),
                        video_width: None,
                        video_height: None,
                        mime_type: None,
                        transport_state: shared.transport_state.clone(),
                        video_bytes_total: 0,
                        video_packet_count_total: 0,
                        audio_bytes_total: shared.inbound_audio_bytes_total,
                        observed_at_ms: now_ms,
                    });
                }
            });
        }
    });
}

fn update_video_track_status(runtime_stats: &RuntimeStatsSink, status: XbxEngineVideoTrackStatus) {
    let should_publish = runtime_stats
        .read(|shared| shared.latest_video_track_status.as_ref() != Some(&status))
        .unwrap_or(true);
    if should_publish {
        runtime_stats.record_video_track_status(status);
    }
}

fn normalize_remote_track_mime(mime_type: &str) -> Option<String> {
    let normalized = mime_type.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn is_primary_remote_video_track(
    track_kind: RTPCodecType,
    transceiver_kind: Option<RTPCodecType>,
    mime_type: Option<&str>,
) -> bool {
    track_kind == RTPCodecType::Video
        || transceiver_kind == Some(RTPCodecType::Video)
        || mime_type.is_some_and(|mime| mime.starts_with("video/"))
}

pub(crate) fn configure_peer_connection_offer_primitives(
    runtime: &Handle,
    peer_connection: &Arc<RTCPeerConnection>,
) -> Result<(), XbxEngineRuntimeError> {
    runtime.block_on(async {
        let audio = peer_connection
            .add_transceiver_from_kind(
                RTPCodecType::Audio,
                Some(RTCRtpTransceiverInit {
                    direction: RTCRtpTransceiverDirection::Sendrecv,
                    send_encodings: vec![],
                }),
            )
            .await
            .map_err(map_webrtc_error("addAudioTransceiverFailed"))?;

        let video = peer_connection
            .add_transceiver_from_kind(
                RTPCodecType::Video,
                Some(RTCRtpTransceiverInit {
                    direction: RTCRtpTransceiverDirection::Recvonly,
                    send_encodings: vec![],
                }),
            )
            .await
            .map_err(map_webrtc_error("addVideoTransceiverFailed"))?;
        let video_codec_preferences = build_offer_video_codec_preferences();
        video
            .set_codec_preferences(video_codec_preferences)
            .await
            .map_err(map_webrtc_error("setVideoCodecPreferencesFailed"))?;

        crate::xbx_log_debug!(
            "[xbxengine][webrtc-rs] add transceiver kind=audio direction={} current_direction={} mid={}",
            audio.direction(),
            audio.current_direction(),
            audio.mid().as_deref().unwrap_or("")
        );
        crate::xbx_log_debug!(
            "[xbxengine][webrtc-rs] add transceiver kind=video direction={} current_direction={} mid={}",
            video.direction(),
            video.current_direction(),
            video.mid().as_deref().unwrap_or("")
        );

        let transceivers = peer_connection.get_transceivers().await;
        for transceiver in transceivers {
            crate::xbx_log_debug!(
                "[xbxengine][webrtc-rs] transceiver kind={} direction={} current_direction={} mid={}",
                transceiver.kind(),
                transceiver.direction(),
                transceiver.current_direction(),
                transceiver.mid().as_deref().unwrap_or("")
            );
        }

        Ok(())
    })
}

pub(crate) fn configure_owned_nack(
    mut registry: interceptor::registry::Registry,
    media_engine: &mut webrtc::api::media_engine::MediaEngine,
    _interval: std::time::Duration,
    _runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
) -> interceptor::registry::Registry {
    media_engine.register_feedback(
        RTCPFeedback {
            typ: TYPE_RTCP_FB_NACK.to_owned(),
            parameter: "".to_owned(),
        },
        RTPCodecType::Video,
    );
    media_engine.register_feedback(
        RTCPFeedback {
            typ: TYPE_RTCP_FB_NACK.to_owned(),
            parameter: "pli".to_owned(),
        },
        RTPCodecType::Video,
    );

    registry.add(Box::new(interceptor::nack::responder::Responder::builder()));
    registry
}

pub(crate) fn register_owned_h264_codecs(
    media_engine: &mut webrtc::api::media_engine::MediaEngine,
) -> webrtc::error::Result<()> {
    // register_default_codecs 里没有 main(4d) packetization-mode=1 这档，
    // 但我们的 rust-owned offer 会把 4d family 放进协商优先级里。
    // 这里把 offer/primitives 用到的 H.264 family 同步注册进 MediaEngine，
    // 避免出现 SDP 谈成 main，但接收层实际上不认识该 payload/fmtp 的半失效状态。
    for codec in build_offer_video_codec_preferences() {
        media_engine.register_codec(codec, RTPCodecType::Video)?;
    }
    Ok(())
}

pub(crate) fn configure_owned_twcc_receiver(
    registry: &mut interceptor::registry::Registry,
    media_engine: &mut webrtc::api::media_engine::MediaEngine,
    interval: std::time::Duration,
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
) -> webrtc::error::Result<()> {
    media_engine.register_feedback(
        RTCPFeedback {
            typ: TYPE_RTCP_FB_TRANSPORT_CC.to_string(),
            parameter: String::new(),
        },
        RTPCodecType::Video,
    );
    media_engine.register_header_extension(
        RTCRtpHeaderExtensionCapability {
            uri: TRANSPORT_CC_URI.to_string(),
        },
        RTPCodecType::Video,
        None,
    )?;

    media_engine.register_feedback(
        RTCPFeedback {
            typ: TYPE_RTCP_FB_TRANSPORT_CC.to_string(),
            parameter: String::new(),
        },
        RTPCodecType::Audio,
    );
    media_engine.register_header_extension(
        RTCRtpHeaderExtensionCapability {
            uri: TRANSPORT_CC_URI.to_string(),
        },
        RTPCodecType::Audio,
        None,
    )?;

    registry.add(Box::new(OwnedTwccReceiverBuilder::new(
        interval,
        runtime_stats,
    )));
    Ok(())
}

fn build_offer_video_codec_preferences() -> Vec<RTCRtpCodecParameters> {
    build_h264_codec_preferences()
}

fn build_h264_codec_preferences() -> Vec<RTCRtpCodecParameters> {
    let video_rtcp_feedback = vec![
        RTCPFeedback {
            typ: TYPE_RTCP_FB_GOOG_REMB.to_string(),
            parameter: String::new(),
        },
        RTCPFeedback {
            typ: TYPE_RTCP_FB_TRANSPORT_CC.to_string(),
            parameter: String::new(),
        },
        RTCPFeedback {
            typ: TYPE_RTCP_FB_CCM.to_string(),
            parameter: "fir".to_string(),
        },
        RTCPFeedback {
            typ: TYPE_RTCP_FB_NACK.to_string(),
            parameter: String::new(),
        },
        RTCPFeedback {
            typ: TYPE_RTCP_FB_NACK.to_string(),
            parameter: "pli".to_string(),
        },
    ];

    // 按高 -> 主 -> 受限基线 -> 基线排列，确保 peer connection 的默认 offer 回退方向正确。
    let mut codecs = vec![
        RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: MIME_TYPE_H264.to_string(),
                clock_rate: 90_000,
                channels: 0,
                sdp_fmtp_line:
                    "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=640032"
                        .to_string(),
                rtcp_feedback: video_rtcp_feedback.clone(),
            },
            payload_type: 123,
            ..Default::default()
        },
        RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: MIME_TYPE_H264.to_string(),
                clock_rate: 90_000,
                channels: 0,
                sdp_fmtp_line:
                    "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=4d0032"
                        .to_string(),
                rtcp_feedback: video_rtcp_feedback.clone(),
            },
            payload_type: 124,
            ..Default::default()
        },
        RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: MIME_TYPE_H264.to_string(),
                clock_rate: 90_000,
                channels: 0,
                sdp_fmtp_line:
                    "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"
                        .to_string(),
                rtcp_feedback: video_rtcp_feedback,
            },
            payload_type: 125,
            ..Default::default()
        },
        RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: MIME_TYPE_H264.to_string(),
                clock_rate: 90_000,
                channels: 0,
                sdp_fmtp_line:
                    "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42001f"
                        .to_string(),
                rtcp_feedback: vec![
                    RTCPFeedback {
                        typ: TYPE_RTCP_FB_GOOG_REMB.to_string(),
                        parameter: String::new(),
                    },
                    RTCPFeedback {
                        typ: TYPE_RTCP_FB_TRANSPORT_CC.to_string(),
                        parameter: String::new(),
                    },
                    RTCPFeedback {
                        typ: TYPE_RTCP_FB_CCM.to_string(),
                        parameter: "fir".to_string(),
                    },
                    RTCPFeedback {
                        typ: TYPE_RTCP_FB_NACK.to_string(),
                        parameter: String::new(),
                    },
                    RTCPFeedback {
                        typ: TYPE_RTCP_FB_NACK.to_string(),
                        parameter: "pli".to_string(),
                    },
                ],
            },
            payload_type: 102,
            ..Default::default()
        },
    ];

    // 固定协商 RTX。当前只验证最小可用 repair 路径，不再让 RED/ULPFEC 干扰结论。
    codecs.push(RTCRtpCodecParameters {
        capability: RTCRtpCodecCapability {
            mime_type: "video/rtx".to_string(),
            clock_rate: 90_000,
            channels: 0,
            sdp_fmtp_line: "apt=124".to_string(),
            rtcp_feedback: vec![],
        },
        payload_type: 97,
        ..Default::default()
    });

    codecs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_remote_video_track_accepts_video_kind_even_without_mime() {
        assert!(is_primary_remote_video_track(
            RTPCodecType::Video,
            Some(RTPCodecType::Video),
            None,
        ));
    }

    #[test]
    fn primary_remote_video_track_accepts_video_transceiver_when_codec_metadata_is_sparse() {
        assert!(is_primary_remote_video_track(
            RTPCodecType::Unspecified,
            Some(RTPCodecType::Video),
            None,
        ));
    }

    #[test]
    fn primary_remote_video_track_rejects_non_video_tracks() {
        assert!(!is_primary_remote_video_track(
            RTPCodecType::Audio,
            Some(RTPCodecType::Audio),
            Some("audio/opus"),
        ));
    }

    #[test]
    fn h264_codec_preferences_include_main_profile_family() {
        let codecs = build_h264_codec_preferences();
        assert!(codecs.iter().any(|codec| {
            codec.payload_type == 124
                && codec
                    .capability
                    .sdp_fmtp_line
                    .contains("profile-level-id=4d0032")
        }));
    }

    #[test]
    fn offer_video_codec_preferences_always_include_rtx_probe() {
        let codecs = build_h264_codec_preferences();
        assert!(codecs.iter().any(|codec| {
            codec.capability.mime_type.eq_ignore_ascii_case("video/rtx")
                && codec.payload_type == 97
                && codec.capability.sdp_fmtp_line == "apt=124"
        }));
        assert!(!codecs
            .iter()
            .any(|codec| codec.capability.mime_type.eq_ignore_ascii_case("video/red")));
        assert!(!codecs.iter().any(|codec| codec
            .capability
            .mime_type
            .eq_ignore_ascii_case("video/ulpfec")));
    }

    #[test]
    fn offer_video_codec_preferences_keep_red_and_ulpfec_disabled() {
        let codecs = build_h264_codec_preferences();
        assert!(!codecs
            .iter()
            .any(|codec| codec.capability.mime_type.eq_ignore_ascii_case("video/red")));
        assert!(!codecs.iter().any(|codec| codec
            .capability
            .mime_type
            .eq_ignore_ascii_case("video/ulpfec")));
    }
}

pub(crate) fn create_initial_data_channels(
    runtime: &Handle,
    peer_connection: &Arc<RTCPeerConnection>,
    data_channels: &mut BTreeMap<String, Arc<RTCDataChannel>>,
    data_channel_state: Arc<Mutex<XbxDataChannelState>>,
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
) -> Result<(), XbxEngineRuntimeError> {
    let configs = [
        ("input", true, "1.0"),
        ("control", true, "controlV1"),
        ("chat", true, "chatV1"),
        ("message", true, "messageV1"),
    ];

    for (name, ordered, protocol) in configs {
        let channel = runtime.block_on(async {
            peer_connection
                .create_data_channel(
                    name,
                    Some(RTCDataChannelInit {
                        ordered: Some(ordered),
                        protocol: Some(protocol.to_string()),
                        ..Default::default()
                    }),
                )
                .await
                .map_err(map_webrtc_error("createDataChannelFailed"))
        })?;
        crate::xbx_log_debug!(
            "[xbxengine][webrtc-rs] local data channel created label={} protocol={} ordered={}",
            channel.label(),
            channel.protocol(),
            ordered
        );
        data_channels.insert(name.to_string(), channel);
    }

    install_data_channel_contracts(data_channels, data_channel_state, runtime_stats)?;
    Ok(())
}

pub(crate) fn build_rtc_configuration(
    turn_server: Option<&XbxEngineTurnServerDto>,
) -> RTCConfiguration {
    let mut ice_servers = vec![RTCIceServer {
        urls: DEFAULT_ICE_SERVERS
            .iter()
            .map(|url| (*url).to_string())
            .collect(),
        ..Default::default()
    }];
    if let Some(turn_server) = turn_server {
        ice_servers.push(RTCIceServer {
            urls: vec![turn_server.url.clone()],
            username: turn_server.username.clone(),
            credential: turn_server.credential.clone(),
        });
    }
    RTCConfiguration {
        ice_servers,
        ..Default::default()
    }
}

pub(crate) fn map_peer_connection_state(
    state: RTCPeerConnectionState,
) -> XbxEngineTransportStateDto {
    match state {
        RTCPeerConnectionState::Connected => XbxEngineTransportStateDto::Connected,
        RTCPeerConnectionState::Connecting => XbxEngineTransportStateDto::Connecting,
        RTCPeerConnectionState::Disconnected => XbxEngineTransportStateDto::Disconnected,
        RTCPeerConnectionState::Failed => XbxEngineTransportStateDto::Failed,
        RTCPeerConnectionState::Closed => XbxEngineTransportStateDto::Closed,
        _ => XbxEngineTransportStateDto::New,
    }
}

pub(crate) fn normalize_remote_ice_candidate(candidate: &str) -> Option<String> {
    let trimmed = candidate.trim();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed == "a=end-of-candidates" || trimmed == "end-of-candidates" {
        return None;
    }

    if trimmed.contains("UDP") && trimmed.contains("tcptype") {
        return None;
    }

    Some(trimmed.strip_prefix("a=").unwrap_or(trimmed).to_string())
}

pub(crate) fn now_ms_f64() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}

pub(crate) fn map_webrtc_error(
    prefix: impl Into<String>,
) -> impl FnOnce(webrtc::Error) -> XbxEngineRuntimeError {
    let prefix = prefix.into();
    move |error| XbxEngineRuntimeError::new(format!("{prefix}:{error}"))
}
