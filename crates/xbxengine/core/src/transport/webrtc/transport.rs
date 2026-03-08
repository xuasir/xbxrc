use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};

use tokio::runtime::Handle;
use tokio::sync::mpsc;
use webrtc::{
    api::{
        interceptor_registry::register_default_interceptors,
        media_engine::{MediaEngine, MIME_TYPE_H264},
        APIBuilder,
    },
    data_channel::{data_channel_init::RTCDataChannelInit, RTCDataChannel},
    ice_transport::{
        ice_candidate::RTCIceCandidate, ice_candidate::RTCIceCandidateInit,
        ice_credential_type::RTCIceCredentialType, ice_server::RTCIceServer,
    },
    peer_connection::{
        configuration::RTCConfiguration, peer_connection_state::RTCPeerConnectionState,
        sdp::session_description::RTCSessionDescription, RTCPeerConnection,
    },
    rtp_transceiver::{
        rtp_codec::{RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType},
        rtp_transceiver_direction::RTCRtpTransceiverDirection,
        RTCPFeedback, RTCRtpTransceiverInit, TYPE_RTCP_FB_CCM, TYPE_RTCP_FB_GOOG_REMB,
        TYPE_RTCP_FB_NACK,
    },
};

use crate::{

    media::video::render::renderer::XbxRenderState,
    transport::adapter::{FrameSource, WebrtcVideoAdapter},
    transport::webrtc::data_channel::{
        install_data_channel_contracts, request_decoder_reset_on_control_channel,
        request_video_keyframe_on_control_channel, XbxDataChannelState,
    },
    transport::webrtc::microphone::XbxMicrophoneSession,
    XbxEngineMediaNegotiationRequest, XbxEngineMediaRuntimeStats,
    XbxEngineRuntimeError, XbxEngineWebRtcRuntimeConfig,
};
use xbxengine_protocol::{
    XbxEngineIceCandidateDto, XbxEngineTransportStateDto, XbxEngineTurnServerDto,
};

const DEFAULT_ICE_SERVERS: [&str; 7] = [
    "stun:worldaz.relay.teams.microsoft.com:3478",
    "stun:stun.l.google.com:19302",
    "stun:stun1.l.google.com:19302",
    "stun:relay1.expressturn.com",
    "stun:relay2.expressturn.com",
    "stun:stun.kinesisvideo.us-east-1.amazonaws.com:443",
    "stun:stun.douyucdn.cn:18000",
];

const VIDEO_CONTROL_WARMUP_MS: f64 = 1_000.0;

#[derive(Default)]
pub(crate) struct XbxTransportState {
    peer_connection: Option<Arc<RTCPeerConnection>>,
    data_channels: BTreeMap<String, Arc<RTCDataChannel>>,
    local_candidates: Arc<Mutex<Vec<XbxEngineIceCandidateDto>>>,
    microphone_session: Option<XbxMicrophoneSession>,
    pub(crate) frame_source_tx: Arc<Mutex<Option<mpsc::Sender<Box<dyn FrameSource>>>>>,
}

impl XbxTransportState {
    pub(crate) fn rebuild_peer_connection(
        &mut self,
        runtime: &Handle,
        request: &XbxEngineMediaNegotiationRequest,
        data_channel_state: Arc<Mutex<XbxDataChannelState>>,
        runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        render_state: Arc<Mutex<XbxRenderState>>,
        webrtc_config: &XbxEngineWebRtcRuntimeConfig,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.stop_peer_connection(runtime);
        self.clear_local_candidates();
        self.data_channels.clear();
        if let Ok(mut stats) = runtime_stats.lock() {
            *stats = XbxEngineMediaRuntimeStats {
                transport_state: XbxEngineTransportStateDto::New,
                ..Default::default()
            };
        }

        let peer_connection = Arc::new(runtime.block_on(async {
            let mut media_engine = MediaEngine::default();
            media_engine
                .register_default_codecs()
                .map_err(map_webrtc_error("registerDefaultCodecsFailed"))?;
            // 注册默认拦截器，确保接收侧具备 RTCP report / TWCC / NACK 反馈能力。
            let interceptor_registry =
                register_default_interceptors(Default::default(), &mut media_engine)
                    .map_err(map_webrtc_error("registerDefaultInterceptorsFailed"))?;
            let api = APIBuilder::new()
                .with_media_engine(media_engine)
                .with_interceptor_registry(interceptor_registry)
                .build();
            api.new_peer_connection(build_rtc_configuration(
                request.session.turn_server.as_ref(),
            ))
            .await
            .map_err(map_webrtc_error("createPeerConnectionFailed"))
        })?);

        install_peer_connection_callbacks(
            &peer_connection,
            self.local_candidates.clone(),
            runtime_stats,
            render_state,
            webrtc_config.clone(),
            self.frame_source_tx.clone(),
        );
        configure_peer_connection_offer_primitives(runtime, &peer_connection)?;
        create_initial_data_channels(
            runtime,
            &peer_connection,
            &mut self.data_channels,
            data_channel_state,
        )?;
        self.peer_connection = Some(peer_connection);
        Ok(())
    }

    pub(crate) fn create_offer(&self, runtime: &Handle) -> Result<String, XbxEngineRuntimeError> {
        let peer_connection = self.require_peer_connection()?;
        let local_offer = runtime.block_on(async {
            let mut gather_complete = peer_connection.gathering_complete_promise().await;
            let offer = peer_connection
                .create_offer(None)
                .await
                .map_err(map_webrtc_error("createOfferFailed"))?;
            peer_connection
                .set_local_description(offer)
                .await
                .map_err(map_webrtc_error("setLocalDescriptionFailed"))?;
            let _ = gather_complete.recv().await;
            peer_connection
                .local_description()
                .await
                .ok_or_else(|| XbxEngineRuntimeError::new("localDescriptionMissing"))
        })?;
        let patched_offer_sdp = apply_offer_policy_contract(&local_offer.sdp);
        validate_local_offer_sdp(&patched_offer_sdp)?;
        crate::xbx_log_info!(
            "[xbxengine][webrtc-rs] local offer created {}",
            summarize_sdp(&patched_offer_sdp)
        );
        Ok(patched_offer_sdp)
    }

    pub(crate) fn apply_remote_description(
        &self,
        runtime: &Handle,
        answer_sdp: &str,
        remote_candidates: &[XbxEngineIceCandidateDto],
    ) -> Result<(), XbxEngineRuntimeError> {
        let peer_connection = self.require_peer_connection()?;
        runtime.block_on(async {
            peer_connection
                .set_remote_description(
                    RTCSessionDescription::answer(answer_sdp.to_string())
                        .map_err(map_webrtc_error("buildRemoteAnswerFailed"))?,
                )
                .await
                .map_err(map_webrtc_error("setRemoteDescriptionFailed"))?;
            crate::xbx_log_info!(
                "[xbxengine][webrtc-rs] remote answer applied {}",
                summarize_sdp(answer_sdp)
            );

            for candidate in remote_candidates {
                let Some(normalized_candidate) =
                    normalize_remote_ice_candidate(&candidate.candidate)
                else {
                    crate::xbx_log_debug!(
                        "[xbxengine][webrtc-rs] remote ice candidate skipped raw={}",
                        candidate.candidate.chars().take(160).collect::<String>()
                    );
                    continue;
                };
                peer_connection
                    .add_ice_candidate(RTCIceCandidateInit {
                        candidate: normalized_candidate,
                        sdp_mid: candidate.sdp_mid.clone(),
                        sdp_mline_index: candidate.sdp_m_line_index,
                        username_fragment: None,
                    })
                    .await
                    .map_err(map_webrtc_error("addRemoteIceCandidateFailed"))?;
                crate::xbx_log_debug!(
                    "[xbxengine][webrtc-rs] remote ice candidate applied mline={} mid={} raw={}",
                    candidate.sdp_m_line_index.unwrap_or_default(),
                    candidate.sdp_mid.as_deref().unwrap_or(""),
                    candidate.candidate.chars().take(160).collect::<String>()
                );
            }
            Ok(())
        })
    }

    pub(crate) fn local_candidates_snapshot(&self) -> Vec<XbxEngineIceCandidateDto> {
        self.local_candidates
            .lock()
            .ok()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub(crate) fn request_video_keyframe(
        &self,
        runtime: &Handle,
    ) -> Result<(), XbxEngineRuntimeError> {
        let Some(control_channel) = self.data_channels.get("control").cloned() else {
            return Ok(());
        };

        runtime
            .block_on(async { request_video_keyframe_on_control_channel(&control_channel).await })
    }

    pub(crate) fn request_decoder_reset(
        &self,
        runtime: &Handle,
    ) -> Result<(), XbxEngineRuntimeError> {
        let Some(control_channel) = self.data_channels.get("control").cloned() else {
            return Ok(());
        };
        runtime.block_on(async { request_decoder_reset_on_control_channel(&control_channel).await })
    }

    pub(crate) fn set_microphone_capturing(
        &mut self,
        runtime: &Handle,
        capturing: bool,
    ) -> Result<(), XbxEngineRuntimeError> {
        let peer_connection = self.require_peer_connection()?;
        if capturing {
            if self.microphone_session.is_some() {
                return Ok(());
            }
            let session = XbxMicrophoneSession::start(runtime, &peer_connection)?;
            self.microphone_session = Some(session);
            return Ok(());
        }

        if let Some(session) = self.microphone_session.take() {
            session.stop(runtime, &peer_connection)?;
        }
        Ok(())
    }

    pub(crate) fn stop_peer_connection(&mut self, runtime: &Handle) {
        if let (Some(session), Some(peer_connection)) = (
            self.microphone_session.take(),
            self.peer_connection.as_ref().cloned(),
        ) {
            if let Err(error) = session.stop(runtime, &peer_connection) {
                crate::xbx_log_error!(
                    "[xbxengine][webrtc-rs][mic] stop failed during shutdown: {error}"
                );
            }
        }
        if let Some(peer_connection) = self.peer_connection.take() {
            let _ = runtime.block_on(async { peer_connection.close().await });
        }
        self.data_channels.clear();
        // Warning: Do NOT take frame_source_tx here, rebuild_peer_connection calls stop_peer_connection 
        // and we want the same tx to be used for the next connection.
    }

    fn clear_local_candidates(&self) {
        if let Ok(mut candidates) = self.local_candidates.lock() {
            candidates.clear();
        }
    }

    fn require_peer_connection(&self) -> Result<Arc<RTCPeerConnection>, XbxEngineRuntimeError> {
        self.peer_connection
            .as_ref()
            .cloned()
            .ok_or_else(|| XbxEngineRuntimeError::new("xbxEnginePeerConnectionMissing"))
    }
}

fn install_peer_connection_callbacks(
    peer_connection: &Arc<RTCPeerConnection>,
    local_candidates: Arc<Mutex<Vec<XbxEngineIceCandidateDto>>>,
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    _render_state: Arc<Mutex<XbxRenderState>>,
    webrtc_config: XbxEngineWebRtcRuntimeConfig,
    frame_source_tx: Arc<Mutex<Option<mpsc::Sender<Box<dyn FrameSource>>>>>,
) {
    peer_connection.on_ice_candidate(Box::new(move |candidate: Option<RTCIceCandidate>| {
        let local_candidates = local_candidates.clone();
        Box::pin(async move {
            let Some(candidate) = candidate else {
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
            if let Ok(mut stats) = runtime_stats.lock() {
                stats.transport_state = map_peer_connection_state(state);
            }
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

    peer_connection.on_track(Box::new(move |track, _, _transceiver| {
        let frame_source_tx = frame_source_tx.clone();
        let pc_captured = peer_connection_for_track.clone(); 
        let config_captured = webrtc_config_for_track.clone();
        let _stats_captured = runtime_stats_for_track.clone();
        
        Box::pin(async move {
            crate::xbx_log_info!("[xbxengine][webrtc-rs] ON_TRACK received: kind={} ssrc={} mime={}", track.kind(), track.ssrc(), track.codec().capability.mime_type);
            crate::xbx_log_debug!("[xbxengine][webrtc-rs] remote track kind={} mime={}", track.kind(), track.codec().capability.mime_type);
            
            let is_video = track.kind() == webrtc::rtp_transceiver::rtp_codec::RTPCodecType::Video;
            let video_mime_type = track.codec().capability.mime_type.to_ascii_lowercase();
            let is_primary_video_track = is_video && video_mime_type == "video/h264";
            
            if is_primary_video_track {
                let jitter_buffer_size = config_captured.video_pipeline.jitter_buffer_max_packets;
                let idle_timeout = std::time::Duration::from_millis(config_captured.video_pipeline.idle_timeout_ms);
                
                crate::xbx_log_info!(
                    "[xbxengine][webrtc-rs] mounting video track with mode={:?} jitter_buffer={} idle_timeout={:?}",
                    config_captured.mode, jitter_buffer_size, idle_timeout
                );

                let adapter = WebrtcVideoAdapter::new(track.clone(), jitter_buffer_size, idle_timeout);
                let source: Box<dyn FrameSource> = Box::new(adapter);
                if let Ok(guard) = frame_source_tx.lock() {
                    if let Some(tx) = guard.as_ref() {
                        if let Err(e) = tx.try_send(source) {
                            crate::xbx_log_error!("[xbxengine][webrtc-rs] Failed to mount new video track: {}", e);
                        }
                    } else {
                        crate::xbx_log_error!("[xbxengine][webrtc-rs] frame_source_tx is None! Supervisor task is dead?");
                    }
                }
                
                // --- 增加对该主视频轨道的指标轮询监控 ---
                let stats_track = track.clone();
                let pc_for_stats = pc_captured;
                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
                    let mut last_bytes_received = 0;
                    let mut tick_count = 0u64;
                    loop {
                        interval.tick().await;
                        tick_count += 1;
                        // 这里我们使用 pc 统一拉取 stats
                        let stats = pc_for_stats.get_stats().await;
                        let mut current_bytes = 0;
                        let mut packets_received = 0u64;
                        let mut packets_lost = 0i64;
                        let mut rtt = 0.0f64;
                        let mut rtt_source: Option<&'static str> = None;
                        let mut fraction_lost = 0.0f64;
                        
                        let mut report_counts = std::collections::HashMap::<String, usize>::new();
                        // ICE 层可用带宽（nominated pair 的 available_outgoing_bitrate，
                        // 基于 REMB 信令计算，反映网络容量上限）
                        let mut avail_bps = 0.0f64;
                        let mut avail_in_bps = 0.0f64;
                        for (_id, report) in stats.reports.iter() {
                            let type_name = format!("{:?}", report).split('(').next().unwrap_or("Unknown").to_string();
                            *report_counts.entry(type_name).or_insert(0) += 1;

                            match report {
                                webrtc::stats::StatsReportType::InboundRTP(inbound) => {
                                    if inbound.ssrc == stats_track.ssrc() {
                                        current_bytes = inbound.bytes_received;
                                        packets_received = inbound.packets_received;
                                        if tick_count % 50 == 0 {
                                            crate::xbx_log_info!("[xbxengine][stats-debug] inbound stats ssrc={}: {:?}", inbound.ssrc, inbound);
                                        }
                                    }
                                }
                                webrtc::stats::StatsReportType::CandidatePair(pair) => {
                                    if pair.available_outgoing_bitrate > avail_bps {
                                        avail_bps = pair.available_outgoing_bitrate;
                                    }
                                    if pair.available_incoming_bitrate > avail_in_bps {
                                        avail_in_bps = pair.available_incoming_bitrate;
                                    }
                                    // RemoteInboundRTP 在部分链路下不可用，RTT 回退到已提名 candidate pair。
                                    if pair.nominated && pair.current_round_trip_time > 0.0 {
                                        rtt = pair.current_round_trip_time;
                                        rtt_source = Some("candidate-pair");
                                    }
                                }
                                webrtc::stats::StatsReportType::RemoteInboundRTP(remote_inbound) => {
                                    if remote_inbound.ssrc == stats_track.ssrc() {
                                        packets_lost = remote_inbound.packets_lost;
                                        fraction_lost = remote_inbound.fraction_lost;
                                        rtt = remote_inbound.round_trip_time.unwrap_or(0.0);
                                        rtt_source = Some("remote-inbound");
                                        if tick_count % 50 == 0 {
                                            crate::xbx_log_info!("[xbxengine][stats-debug] remote inbound stats ssrc={}: {:?}", remote_inbound.ssrc, remote_inbound);
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        
                        if tick_count % 50 == 0 {
                            crate::xbx_log_info!("[xbxengine][stats-debug] report counts: {:?}", report_counts);
                        }
                        
                        // webrtc-rs 暂未在 InboundRTP 暴露 packetsLost，只有 fraction_lost 时做估算显示。
                        if packets_lost == 0 && fraction_lost > 0.0 && packets_received > 0 {
                            packets_lost = (fraction_lost * packets_received as f64).round() as i64;
                        }

                        // 写回共享 stats
                        if let Ok(mut shared) = _stats_captured.lock() {
                            // Ceiling: 优先用 BWE (avail_bps)，如果为 0 则回退到 forced_remb_kbps
                            shared.video_remb_bps = if avail_bps > 0.0 { 
                                Some(avail_bps as u32) 
                            } else { 
                                config_captured.forced_remb_kbps.map(|k| k * 1000) 
                            };
                            shared.video_rtt_ms = if rtt > 0.0 { Some(rtt * 1000.0) } else { None };
                            shared.video_rtt_source = rtt_source.map(str::to_string);
                            shared.inbound_video_loss_ratio_5s = fraction_lost;
                        }

                        // --- GCC 欺骗与码率锁死实装 ---
                        // 根据配置注入 REMB 值。只有配置了 forced_remb_kbps 且未开启 adaptive 时才强制注入。
                        let mut inject_result = Ok(0usize);
                        if let Some(kbps) = config_captured.forced_remb_kbps {
                            use webrtc::rtcp::payload_feedbacks::receiver_estimated_maximum_bitrate::*;
                            let remb = ReceiverEstimatedMaximumBitrate {
                                bitrate: (kbps as f32) * 1000.0, 
                                ssrcs: vec![stats_track.ssrc()],
                                ..Default::default()
                            };
                            inject_result = pc_for_stats.write_rtcp(&[Box::new(remb)]).await;
                        }
                        
                        let delta_bytes = current_bytes.saturating_sub(last_bytes_received);
                        last_bytes_received = current_bytes;
                        let actual_kbps = (delta_bytes * 8) as f64 / 1000.0;
                        
                        // 逻辑：如果 avail_bps 为 0，日志中显示强制码率值，以防用户困惑
                        let display_avail_kbps = if avail_bps > 0.0 {
                            avail_bps / 1000.0
                        } else {
                            config_captured.forced_remb_kbps.map(|k| k as f64).unwrap_or(0.0)
                        };
                        let avail_in_kbps = avail_in_bps / 1000.0;
                        
                        // 定期打印注入状态确认
                        if tick_count % 30 == 0 && config_captured.forced_remb_kbps.is_some() {
                             if inject_result.is_ok() {
                                 crate::xbx_log_info!(
                                     "[xbxengine][Deception] {:?}bps REMB is being injected (mode={:?})", 
                                     config_captured.forced_remb_kbps.unwrap() * 1000,
                                     config_captured.mode
                                 );
                             } else {
                                 crate::xbx_log_warn!("[xbxengine][Deception] REMB injection failed: {:?}", inject_result.err());
                             }
                        }

                        crate::xbx_log_info!(
                            "[NetworkStats] Video: {:.0} Kbps | Ceiling: {:.0} Kbps (in:{:.0}) | Lost: {} | RTT: {:.1}ms",
                            actual_kbps,
                            display_avail_kbps,
                            avail_in_kbps,
                            packets_lost,
                            rtt * 1000.0
                        );
                    }
                });

            } else {
                // Not primary video, just read to drain
                tokio::spawn(async move {
                    while let Ok(_) = track.read_rtp().await {}
                });
            }
        })
    }));
}

fn configure_peer_connection_offer_primitives(
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
        video
            .set_codec_preferences(build_h264_codec_preferences())
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

fn build_h264_codec_preferences() -> Vec<RTCRtpCodecParameters> {
    let video_rtcp_feedback = vec![
        RTCPFeedback {
            typ: TYPE_RTCP_FB_GOOG_REMB.to_string(),
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

    vec![
        RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: MIME_TYPE_H264.to_string(),
                clock_rate: 90_000,
                channels: 0,
                sdp_fmtp_line:
                    "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42001f"
                        .to_string(),
                rtcp_feedback: video_rtcp_feedback.clone(),
            },
            payload_type: 102,
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
                rtcp_feedback: video_rtcp_feedback.clone(),
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
                    "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=640032"
                        .to_string(),
                rtcp_feedback: video_rtcp_feedback,
            },
            payload_type: 123,
            ..Default::default()
        },
    ]
}

fn create_initial_data_channels(
    runtime: &Handle,
    peer_connection: &Arc<RTCPeerConnection>,
    data_channels: &mut BTreeMap<String, Arc<RTCDataChannel>>,
    data_channel_state: Arc<Mutex<XbxDataChannelState>>,
) -> Result<(), XbxEngineRuntimeError> {
    // 硬编码创建标准通道，取消对 network_profile 的依赖。
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

    install_data_channel_contracts(data_channels, data_channel_state)?;
    Ok(())
}

fn build_rtc_configuration(turn_server: Option<&XbxEngineTurnServerDto>) -> RTCConfiguration {
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
            credential_type: RTCIceCredentialType::Password,
        });
    }
    RTCConfiguration {
        ice_servers,
        ..Default::default()
    }
}

// 简化后的 SDP 合约应用：注入带宽行并补充音频立体声支持。
fn apply_offer_policy_contract(offer_sdp: &str) -> String {
    // 默认使用 50Mbps 作为 AS 限制 (1080p60 Peak)
    let with_video_bitrate = set_media_bitrate_as(offer_sdp, "video", 50_000);
    let with_audio_stereo =
        with_video_bitrate.replace("useinbandfec=1", "useinbandfec=1; stereo=1");
    patch_video_fmtp_constraints(&with_audio_stereo)
}

fn set_media_bitrate_as(offer_sdp: &str, media: &str, bitrate: u32) -> String {
    let mut lines = offer_sdp
        .split("\r\n")
        .map(ToOwned::to_owned)
        .collect::<Vec<String>>();
    let mut section_start = 0usize;
    let media_prefix = format!("m={media}");
    let bitrate_line = format!("b=AS:{bitrate}");

    while section_start < lines.len() {
        if !lines[section_start].starts_with(&media_prefix) {
            section_start += 1;
            continue;
        }

        let mut section_end = section_start + 1;
        while section_end < lines.len() && !lines[section_end].starts_with("m=") {
            section_end += 1;
        }

        let mut replaced = false;
        for index in section_start + 1..section_end {
            if lines[index].starts_with("b=AS:") {
                lines[index] = bitrate_line.clone();
                replaced = true;
                break;
            }
        }

        if !replaced {
            let mut insert_at = section_start + 1;
            while insert_at < section_end
                && (lines[insert_at].starts_with("i=") || lines[insert_at].starts_with("c="))
            {
                insert_at += 1;
            }
            lines.insert(insert_at, bitrate_line.clone());
            section_end += 1;
        }

        section_start = section_end;
    }

    lines.join("\r\n")
}

fn patch_video_fmtp_constraints(offer_sdp: &str) -> String {
    let mut lines = offer_sdp
        .split("\r\n")
        .map(ToOwned::to_owned)
        .collect::<Vec<String>>();
    let mut section_start = 0usize;

    // 默认分辨率约束：1080p 相关宏块数
    let max_frame_size = 8160; // (1920/16) * (1080/16) 向上取整宏块

    while section_start < lines.len() {
        if !lines[section_start].starts_with("m=video ") {
            section_start += 1;
            continue;
        }

        let mut section_end = section_start + 1;
        while section_end < lines.len() && !lines[section_end].starts_with("m=") {
            section_end += 1;
        }

        let h264_payload_types = collect_h264_payload_types(&lines[section_start..section_end]);
        for index in section_start + 1..section_end {
            let Some(payload_type) = extract_fmtp_payload_type(&lines[index]) else {
                continue;
            };
            if !h264_payload_types.contains(payload_type) {
                continue;
            }
            lines[index] = upsert_fmtp_constraints(
                &lines[index],
                &[
                    ("x-google-min-bitrate", "5000".to_string()),
                    ("x-google-start-bitrate", "20000".to_string()),
                    ("x-google-max-bitrate", "50000".to_string()),
                    ("max-fs", max_frame_size.to_string()),
                    ("max-fr", "60".to_string()),
                ],
            );
        }

        section_start = section_end;
    }

    lines.join("\r\n")
}

fn collect_video_payload_types(video_media_line: &str) -> HashSet<String> {
    let mut parts = video_media_line.split_whitespace();
    let _ = parts.next();
    let _ = parts.next();
    let _ = parts.next();
    parts.map(ToOwned::to_owned).collect()
}

fn collect_h264_payload_types(video_section_lines: &[String]) -> HashSet<String> {
    let Some(video_media_line) = video_section_lines.first() else {
        return HashSet::new();
    };
    let video_payload_types = collect_video_payload_types(video_media_line);
    let mut h264_payload_types = HashSet::new();
    for line in video_section_lines.iter().skip(1) {
        let Some(rest) = line.strip_prefix("a=rtpmap:") else {
            continue;
        };
        let Some(space_index) = rest.find(char::is_whitespace) else {
            continue;
        };
        let payload_type = &rest[..space_index];
        if !video_payload_types.contains(payload_type) {
            continue;
        }
        let codec_name = rest[space_index + 1..]
            .split('/')
            .next()
            .map(str::trim)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if codec_name == "h264" {
            h264_payload_types.insert(payload_type.to_string());
        }
    }
    h264_payload_types
}

fn extract_fmtp_payload_type(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("a=fmtp:")?;
    let payload_end = rest
        .find(|character: char| character.is_whitespace())
        .unwrap_or(rest.len());
    if payload_end == 0 {
        return None;
    }
    Some(&rest[..payload_end])
}

fn upsert_fmtp_constraints(line: &str, entries: &[(&str, String)]) -> String {
    let Some(rest) = line.strip_prefix("a=fmtp:") else {
        return line.to_string();
    };
    let Some(space_index) = rest.find(char::is_whitespace) else {
        return line.to_string();
    };

    let payload_type = &rest[..space_index];
    let params = rest[space_index + 1..]
        .split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<&str>>();

    let mut normalized = params
        .into_iter()
        .map(ToOwned::to_owned)
        .collect::<Vec<String>>();

    for (key, value) in entries {
        let pattern = format!("{key}=");
        if let Some(index) = normalized
            .iter()
            .position(|part| part.to_ascii_lowercase().starts_with(&pattern))
        {
            normalized[index] = format!("{key}={value}");
        } else {
            normalized.push(format!("{key}={value}"));
        }
    }

    format!("a=fmtp:{payload_type} {}", normalized.join(";"))
}

fn map_peer_connection_state(state: RTCPeerConnectionState) -> XbxEngineTransportStateDto {
    match state {
        RTCPeerConnectionState::Connected => XbxEngineTransportStateDto::Connected,
        RTCPeerConnectionState::Connecting => XbxEngineTransportStateDto::Connecting,
        RTCPeerConnectionState::Disconnected => XbxEngineTransportStateDto::Disconnected,
        RTCPeerConnectionState::Failed => XbxEngineTransportStateDto::Failed,
        RTCPeerConnectionState::Closed => XbxEngineTransportStateDto::Closed,
        _ => XbxEngineTransportStateDto::New,
    }
}

fn validate_local_offer_sdp(offer_sdp: &str) -> Result<(), XbxEngineRuntimeError> {
    let has_audio = offer_sdp.contains("\r\nm=audio ") || offer_sdp.starts_with("m=audio ");
    let has_video = offer_sdp.contains("\r\nm=video ") || offer_sdp.starts_with("m=video ");
    let has_application =
        offer_sdp.contains("\r\nm=application ") || offer_sdp.starts_with("m=application ");
    if has_audio && has_video && has_application {
        return Ok(());
    }
    Err(XbxEngineRuntimeError::new(format!(
        "invalidLocalOfferSdp:audio={has_audio}:video={has_video}:application={has_application}:preview={}",
        offer_sdp.replace("\r\n", " | ").chars().take(320).collect::<String>()
    )))
}

fn normalize_remote_ice_candidate(candidate: &str) -> Option<String> {
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
fn summarize_sdp(sdp: &str) -> String {
    format!(
        "audio={} video={} application={} len={} preview={}",
        sdp.contains("\r\nm=audio ") || sdp.starts_with("m=audio "),
        sdp.contains("\r\nm=video ") || sdp.starts_with("m=video "),
        sdp.contains("\r\nm=application ") || sdp.starts_with("m=application "),
        sdp.len(),
        sdp.replace("\r\n", " | ")
            .chars()
            .take(240)
            .collect::<String>()
    )
}

fn now_ms_f64() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}

fn map_webrtc_error(
    prefix: impl Into<String>,
) -> impl FnOnce(webrtc::Error) -> XbxEngineRuntimeError {
    let prefix = prefix.into();
    move |error| XbxEngineRuntimeError::new(format!("{prefix}:{error}"))
}
