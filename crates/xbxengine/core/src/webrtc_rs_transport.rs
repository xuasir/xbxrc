use std::{
    collections::{BTreeMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use tokio::runtime::Runtime;
use tokio::time::{interval, Duration};
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
    rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication,
    rtcp::payload_feedbacks::receiver_estimated_maximum_bitrate::ReceiverEstimatedMaximumBitrate,
    rtcp::transport_feedbacks::transport_layer_nack::{
        nack_pairs_from_sequence_numbers, TransportLayerNack,
    },
    rtp_transceiver::{
        rtp_codec::{RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType},
        rtp_transceiver_direction::RTCRtpTransceiverDirection,
        RTCPFeedback, RTCRtpTransceiverInit, TYPE_RTCP_FB_CCM, TYPE_RTCP_FB_GOOG_REMB,
        TYPE_RTCP_FB_NACK,
    },
    stats::{ICECandidatePairStats, StatsReportType},
};

use crate::{
    network_profile::STREAM_DATA_CHANNEL_PROFILES,
    webrtc_rs_data_channel::{
        install_data_channel_contracts, request_video_keyframe_on_control_channel,
        WebRtcRsDataChannelState,
    },
    webrtc_rs_h264_resolution::H264ResolutionTracker,
    webrtc_rs_negotiation_profile::current_webrtc_rs_negotiation_profile,
    webrtc_rs_video_pipeline::{WebRtcRsVideoPipelineConfig, WebRtcRsVideoPipelineState},
    XbxEngineMediaNegotiationRequest, XbxEngineMediaRuntimeStats,
    XbxEngineRttDiagnosticsRuntimeConfig, XbxEngineRuntimeError, XbxEngineWebRtcRuntimeConfig,
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

const FORCE_REMB_SENDER_SSRC: u32 = 1;
const STATS_LOOP_INTERVAL_MS: u64 = 1_000;
const CONTROL_LOOP_INTERVAL_MS: u64 = 1_000;

#[derive(Default)]
pub(crate) struct WebRtcRsTransportState {
    peer_connection: Option<Arc<RTCPeerConnection>>,
    data_channels: BTreeMap<String, Arc<RTCDataChannel>>,
    local_candidates: Arc<Mutex<Vec<XbxEngineIceCandidateDto>>>,
}

impl WebRtcRsTransportState {
    pub(crate) fn rebuild_peer_connection(
        &mut self,
        runtime: &Runtime,
        request: &XbxEngineMediaNegotiationRequest,
        data_channel_state: Arc<Mutex<WebRtcRsDataChannelState>>,
        runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        webrtc_config: &XbxEngineWebRtcRuntimeConfig,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.stop_peer_connection(runtime);
        self.clear_local_candidates();
        self.data_channels.clear();
        *runtime_stats.lock().expect("lock runtime stats") = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::New,
            ..Default::default()
        };

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
            webrtc_config.clone(),
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

    pub(crate) fn create_offer(&self, runtime: &Runtime) -> Result<String, XbxEngineRuntimeError> {
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
        eprintln!(
            "[xbxengine][webrtc-rs] local offer created {}",
            summarize_sdp(&patched_offer_sdp)
        );
        Ok(patched_offer_sdp)
    }

    pub(crate) fn apply_remote_description(
        &self,
        runtime: &Runtime,
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
            eprintln!(
                "[xbxengine][webrtc-rs] remote answer applied {}",
                summarize_sdp(answer_sdp)
            );

            for candidate in remote_candidates {
                let Some(normalized_candidate) =
                    normalize_remote_ice_candidate(&candidate.candidate)
                else {
                    eprintln!(
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
                eprintln!(
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
            .expect("lock local candidates")
            .clone()
    }

    pub(crate) fn request_video_keyframe(
        &self,
        runtime: &Runtime,
    ) -> Result<(), XbxEngineRuntimeError> {
        let Some(control_channel) = self.data_channels.get("control").cloned() else {
            return Ok(());
        };

        runtime
            .block_on(async { request_video_keyframe_on_control_channel(&control_channel).await })
    }

    pub(crate) fn stop_peer_connection(&mut self, runtime: &Runtime) {
        if let Some(peer_connection) = self.peer_connection.take() {
            let _ = runtime.block_on(async { peer_connection.close().await });
        }
        self.data_channels.clear();
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
    webrtc_config: XbxEngineWebRtcRuntimeConfig,
) {
    let active_video_controller_ssrc: Arc<Mutex<Option<u32>>> = Arc::new(Mutex::new(None));
    let active_video_pipeline: Arc<Mutex<Option<Arc<Mutex<WebRtcRsVideoPipelineState>>>>> =
        Arc::new(Mutex::new(None));
    peer_connection.on_ice_candidate(Box::new(move |candidate: Option<RTCIceCandidate>| {
        let local_candidates = local_candidates.clone();
        Box::pin(async move {
            let Some(candidate) = candidate else {
                eprintln!("[xbxengine][webrtc-rs] local ice gathering complete");
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
                        eprintln!(
                            "[xbxengine][webrtc-rs] local ice candidate gathered mline={} total={}",
                            list.last()
                                .and_then(|value| value.sdp_m_line_index)
                                .unwrap_or_default(),
                            list.len()
                        );
                    }
                }
                Err(error) => {
                    eprintln!(
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
            eprintln!("[xbxengine][webrtc-rs] peer connection state={state}");
        })
    }));

    peer_connection.on_data_channel(Box::new(|channel| {
        Box::pin(async move {
            eprintln!(
                "[xbxengine][webrtc-rs] remote data channel label={} protocol={} state={:?}",
                channel.label(),
                channel.protocol(),
                channel.ready_state()
            );
        })
    }));

    let peer_connection_for_track = peer_connection.clone();
    peer_connection.on_track(Box::new(move |track, _, transceiver| {
        let runtime_stats = runtime_stats.clone();
        let peer_connection = peer_connection_for_track.clone();
        let webrtc_config = webrtc_config.clone();
        let active_video_controller_ssrc = active_video_controller_ssrc.clone();
        let active_video_pipeline = active_video_pipeline.clone();
        Box::pin(async move {
            eprintln!(
                "[xbxengine][webrtc-rs] remote track kind={} mid={} stream_id={} track_id={} payload_type={} mime={}",
                track.kind(),
                transceiver.mid().as_deref().unwrap_or(""),
                track.stream_id(),
                track.id(),
                track.payload_type(),
                track.codec().capability.mime_type
            );

            let is_video = track.kind() == RTPCodecType::Video;
            let is_audio = track.kind() == RTPCodecType::Audio;
            let video_mime_type = track.codec().capability.mime_type.to_ascii_lowercase();
            let supports_video_codec = !is_video || video_mime_type == "video/h264";
            // 只让主视频编码轨进入三层管线，避免 RTX/FEC 轨污染丢包和反馈控制。
            let is_primary_video_track = is_video && supports_video_codec;
            let mut packet_count = 0u64;
            let mut h264_resolution_tracker = H264ResolutionTracker::default();
            let video_pipeline = Arc::new(Mutex::new(WebRtcRsVideoPipelineState::new(
                webrtc_config.forced_remb_kbps,
                webrtc_config.adaptive_remb_enabled,
                WebRtcRsVideoPipelineConfig::from_runtime_config(&webrtc_config.video_pipeline),
            )));
            let track_mid = transceiver.mid().as_deref().unwrap_or("").to_string();
            let track_id = track.id();
            let track_mime = track.codec().capability.mime_type.clone();
            let track_payload_type = track.payload_type();
            let track_ssrc = track.ssrc();
            let is_primary_video_controller = is_primary_video_track
                && try_acquire_video_control_track(&active_video_controller_ssrc, track_ssrc);
            if is_primary_video_controller {
                if let Ok(mut active_pipeline) = active_video_pipeline.lock() {
                    *active_pipeline = Some(video_pipeline.clone());
                }
            }
            if is_primary_video_track && !is_primary_video_controller {
                eprintln!(
                    "[xbxengine][webrtc-rs] skip secondary h264 control track ssrc={} mid={} track_id={}",
                    track_ssrc, track_mid, track_id
                );
            }
            let mut stats_ticker = interval(Duration::from_millis(STATS_LOOP_INTERVAL_MS));
            let mut control_ticker = interval(Duration::from_millis(CONTROL_LOOP_INTERVAL_MS));
            let last_rtt_diagnostics_logged_at_ms: Arc<Mutex<Option<f64>>> =
                Arc::new(Mutex::new(None));
            let last_control_diagnostics_logged_at_ms: Arc<Mutex<Option<f64>>> =
                Arc::new(Mutex::new(None));
            let control_tick_inflight = Arc::new(AtomicBool::new(false));
            let mut track_window_started_at_ms: Option<f64> = None;
            let mut track_window_packet_count: u64 = 0;
            let mut track_window_payload_bytes: u64 = 0;
            let mut pending_inbound_bytes_total: u64 = 0;
            let mut pending_inbound_video_bytes_total: u64 = 0;
            let mut pending_inbound_primary_video_bytes_total: u64 = 0;
            let mut pending_inbound_audio_bytes_total: u64 = 0;
            let mut pending_stream_resolution: Option<(u32, u32)> = None;
            stats_ticker.tick().await;
            control_ticker.tick().await;

            if is_video && !supports_video_codec {
                eprintln!(
                    "[xbxengine][webrtc-rs] skip non-primary video track mime={} payload_type={} track_id={}",
                    track.codec().capability.mime_type,
                    track.payload_type(),
                    track.id()
                );
            }

            loop {
                tokio::select! {
                    packet_result = track.read_rtp() => {
                        let Ok((packet, _)) = packet_result else {
                            break;
                        };
                        packet_count += 1;
                        let now_ms = now_ms_f64();
                        let packet_bytes = packet.payload.len() as u64;
                        track_window_started_at_ms.get_or_insert(now_ms);
                        track_window_packet_count = track_window_packet_count.saturating_add(1);
                        track_window_payload_bytes =
                            track_window_payload_bytes.saturating_add(packet_bytes);
                        pending_inbound_bytes_total =
                            pending_inbound_bytes_total.saturating_add(packet_bytes);
                        if is_video {
                            // 码率口径统计所有视频轨流量（含 RTX/FEC），避免主轨选择影响吞吐观测。
                            pending_inbound_video_bytes_total =
                                pending_inbound_video_bytes_total.saturating_add(packet_bytes);
                            if is_primary_video_controller {
                                // 主显示码率使用主 H264 控制轨口径，避免 RTX/FEC 抬高指标。
                                pending_inbound_primary_video_bytes_total = pending_inbound_primary_video_bytes_total
                                    .saturating_add(packet_bytes);
                            }
                        } else if is_audio {
                            pending_inbound_audio_bytes_total =
                                pending_inbound_audio_bytes_total.saturating_add(packet_bytes);
                        }
                        let mut nack_sequence_numbers = Vec::new();
                        let mut should_request_pli = false;
                        let mut stream_resolution = None;
                        if is_primary_video_controller {
                            if let Ok(mut pipeline) = video_pipeline.lock() {
                                let action = pipeline.on_rtp_packet(
                                    packet.header.sequence_number,
                                    packet.header.timestamp,
                                    now_ms,
                                );
                                should_request_pli = action.request_pli;
                                nack_sequence_numbers = action.nack_sequence_numbers;
                                stream_resolution = h264_resolution_tracker
                                    .ingest_rtp_payload(packet.header.timestamp, &packet.payload);
                            }
                        }
                        if is_primary_video_controller {
                                if let Some(resolution) = stream_resolution {
                                    pending_stream_resolution = Some(resolution);
                                }
                        } else if is_video && video_mime_type == "video/rtx" {
                            // RTX 轨负载前两个字节是原包序号（OSN），用于回冲主轨 loss 统计。
                            if packet.payload.len() >= 2 {
                                let recovered_sequence =
                                    u16::from_be_bytes([packet.payload[0], packet.payload[1]]);
                                let active_pipeline = active_video_pipeline
                                    .lock()
                                    .ok()
                                    .and_then(|guard| guard.as_ref().cloned());
                                if let Some(active_pipeline) = active_pipeline {
                                    if let Ok(mut pipeline) = active_pipeline.lock() {
                                        pipeline.on_recovered_sequence(recovered_sequence, now_ms);
                                    }
                                }
                            }
                        }

                        if !nack_sequence_numbers.is_empty() {
                            // NACK 发送改为异步下发，避免阻塞 read_rtp 热路径导致自造丢包。
                            let peer_connection = peer_connection.clone();
                            let media_ssrc = track.ssrc();
                            tokio::spawn(async move {
                                if let Err(error) = request_transport_layer_nack(
                                    &peer_connection,
                                    media_ssrc,
                                    &nack_sequence_numbers,
                                )
                                .await
                                {
                                    eprintln!("[xbxengine][webrtc-rs] send nack failed: {error}");
                                }
                            });
                        }

                        if should_request_pli {
                            // PLI 同样异步发送，避免与 RTP drain 互相阻塞。
                            let peer_connection = peer_connection.clone();
                            let media_ssrc = track.ssrc();
                            tokio::spawn(async move {
                                if let Err(error) =
                                    request_video_packet_loss_indication(&peer_connection, media_ssrc).await
                                {
                                    eprintln!("[xbxengine][webrtc-rs] send pli failed: {error}");
                                }
                            });
                        }

                        if is_audio && packet_count == 1 {
                            eprintln!(
                                "[xbxengine][webrtc-rs] first audio packet received seq={} ts={} payload={}",
                                packet.header.sequence_number,
                                packet.header.timestamp,
                                packet.payload.len()
                            );
                        }
                    }
                    _ = stats_ticker.tick() => {
                        let now_ms = now_ms_f64();
                        maybe_log_track_throughput_diagnostics(
                            &webrtc_config.rtt_diagnostics,
                            now_ms,
                            &mut track_window_started_at_ms,
                            &mut track_window_packet_count,
                            &mut track_window_payload_bytes,
                            track.kind(),
                            &track_mime,
                            track_payload_type,
                            track_ssrc,
                            is_primary_video_controller,
                            &track_mid,
                            &track_id,
                        );
                        let mut pipeline_snapshot = None;
                        if is_primary_video_controller {
                            if let Ok(mut pipeline) = video_pipeline.lock() {
                                pipeline.on_stats_tick(now_ms);
                                pipeline_snapshot = Some(snapshot_video_pipeline_runtime_stats(&pipeline));
                            }
                        }
                        if let Ok(mut stats) = runtime_stats.lock() {
                            stats.inbound_bytes_total = stats
                                .inbound_bytes_total
                                .saturating_add(pending_inbound_bytes_total);
                            stats.inbound_video_bytes_total = stats
                                .inbound_video_bytes_total
                                .saturating_add(pending_inbound_video_bytes_total);
                            stats.inbound_primary_video_bytes_total = stats
                                .inbound_primary_video_bytes_total
                                .saturating_add(pending_inbound_primary_video_bytes_total);
                            stats.inbound_audio_bytes_total = stats
                                .inbound_audio_bytes_total
                                .saturating_add(pending_inbound_audio_bytes_total);
                            if let Some((width, height)) = pending_stream_resolution.take() {
                                stats.latest_video_stream_width = Some(width);
                                stats.latest_video_stream_height = Some(height);
                            }
                            if let Some(snapshot) = pipeline_snapshot.as_ref() {
                                apply_video_pipeline_runtime_stats(&mut stats, snapshot);
                            }
                            pending_inbound_bytes_total = 0;
                            pending_inbound_video_bytes_total = 0;
                            pending_inbound_primary_video_bytes_total = 0;
                            pending_inbound_audio_bytes_total = 0;
                        }
                    }
                    _ = control_ticker.tick(), if is_primary_video_controller => {
                        // 控制环独立异步执行，避免 get_stats()/RTCP 发送阻塞 read_rtp 热路径。
                        if control_tick_inflight
                            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                            .is_err()
                        {
                            continue;
                        }
                        let peer_connection = peer_connection.clone();
                        let runtime_stats = runtime_stats.clone();
                        let video_pipeline = video_pipeline.clone();
                        let rtt_diagnostics = webrtc_config.rtt_diagnostics.clone();
                        let forced_remb_kbps = webrtc_config.forced_remb_kbps;
                        let adaptive_remb_enabled = webrtc_config.adaptive_remb_enabled;
                        let last_rtt_diagnostics_logged_at_ms =
                            last_rtt_diagnostics_logged_at_ms.clone();
                        let last_control_diagnostics_logged_at_ms =
                            last_control_diagnostics_logged_at_ms.clone();
                        let control_tick_inflight = control_tick_inflight.clone();
                        tokio::spawn(async move {
                            let now_ms = now_ms_f64();
                            let rtt_sample =
                                sample_peer_connection_rtt(&peer_connection, track_ssrc).await;
                            let mut loss1s_pct = None;
                            let mut loss5s_pct = None;
                            let mut jitter_ms = None;
                            let mut pipeline_rtt_ms = None;
                            let mut pipeline_rtt_source = None;
                            let mut pipeline_remb_bps = None;
                            let mut action_remb_bps = None;
                            let mut action_used_nack_recovery_rtt = false;
                            let mut pipeline_snapshot = None;
                            if let Ok(mut pipeline) = video_pipeline.lock() {
                                let action = pipeline.on_control_tick(rtt_sample.selected_rtt_ms);
                                action_remb_bps = action.remb_bps;
                                action_used_nack_recovery_rtt = action.used_nack_recovery_rtt;
                                pipeline_snapshot = Some(snapshot_video_pipeline_runtime_stats(&pipeline));
                            }
                            if let Ok(mut stats) = runtime_stats.lock() {
                                if let Some(snapshot) = pipeline_snapshot.as_ref() {
                                    apply_video_pipeline_runtime_stats(&mut stats, snapshot);
                                }
                                if let Some(source) = rtt_sample.selected_source {
                                    stats.video_rtt_source = Some(source.to_string());
                                } else if action_used_nack_recovery_rtt {
                                    // stats RTT 取不到时，明确标记为 NACK 恢复回退样本。
                                    stats.video_rtt_source = Some("nack_recovery".to_string());
                                }
                                loss1s_pct = Some(stats.inbound_video_loss_ratio_1s * 100.0);
                                loss5s_pct = Some(stats.inbound_video_loss_ratio_5s * 100.0);
                                jitter_ms = stats.inbound_video_jitter_ms;
                                pipeline_rtt_ms = stats.video_rtt_ms;
                                pipeline_rtt_source = stats.video_rtt_source.clone();
                                pipeline_remb_bps = stats.video_remb_bps;
                            }
                            if let Ok(mut last_logged_at_ms) =
                                last_control_diagnostics_logged_at_ms.lock()
                            {
                                maybe_log_control_diagnostics(
                                    &rtt_diagnostics,
                                    now_ms,
                                    &mut last_logged_at_ms,
                                    track_ssrc,
                                    forced_remb_kbps,
                                    adaptive_remb_enabled,
                                    action_remb_bps.or(pipeline_remb_bps),
                                    loss1s_pct,
                                    loss5s_pct,
                                    jitter_ms,
                                    pipeline_rtt_ms,
                                    pipeline_rtt_source.as_deref(),
                                );
                            }
                            let mut effective_rtt_sample = rtt_sample.clone();
                            if effective_rtt_sample.selected_rtt_ms.is_none()
                                && pipeline_rtt_source.as_deref() == Some("nack_recovery")
                                && pipeline_rtt_ms.is_some()
                            {
                                effective_rtt_sample.selected_rtt_ms = pipeline_rtt_ms;
                                effective_rtt_sample.selected_source = Some("nack_recovery");
                            }
                            if let Ok(mut last_logged_at_ms) =
                                last_rtt_diagnostics_logged_at_ms.lock()
                            {
                                maybe_log_rtt_diagnostics(
                                    &rtt_diagnostics,
                                    &effective_rtt_sample,
                                    now_ms,
                                    &mut last_logged_at_ms,
                                );
                            }
                            if let Some(remb_bps) = action_remb_bps {
                                if let Err(error) = request_receiver_estimated_maximum_bitrate(
                                    &peer_connection,
                                    track_ssrc,
                                    remb_bps,
                                )
                                .await
                                {
                                    eprintln!("[xbxengine][webrtc-rs] send remb failed: {error}");
                                }
                            }
                            control_tick_inflight.store(false, Ordering::Release);
                        });
                    }
                }
            }

            eprintln!(
                "[xbxengine][webrtc-rs] remote track ended kind={} mid={} packets={}",
                track.kind(),
                transceiver.mid().as_deref().unwrap_or(""),
                packet_count
            );
            if is_primary_video_controller {
                release_video_control_track(&active_video_controller_ssrc, track_ssrc);
                if let Ok(mut active_pipeline) = active_video_pipeline.lock() {
                    if active_pipeline
                        .as_ref()
                        .is_some_and(|current| Arc::ptr_eq(current, &video_pipeline))
                    {
                        *active_pipeline = None;
                    }
                }
            }
        })
    }));
}

fn try_acquire_video_control_track(
    active_video_controller_ssrc: &Arc<Mutex<Option<u32>>>,
    ssrc: u32,
) -> bool {
    if let Ok(mut selected) = active_video_controller_ssrc.lock() {
        match *selected {
            Some(current) if current == ssrc => true,
            Some(_) => false,
            None => {
                *selected = Some(ssrc);
                true
            }
        }
    } else {
        false
    }
}

fn release_video_control_track(active_video_controller_ssrc: &Arc<Mutex<Option<u32>>>, ssrc: u32) {
    if let Ok(mut selected) = active_video_controller_ssrc.lock() {
        if selected.as_ref().is_some_and(|current| *current == ssrc) {
            *selected = None;
        }
    }
}

fn snapshot_video_pipeline_runtime_stats(
    pipeline: &WebRtcRsVideoPipelineState,
) -> XbxEngineMediaRuntimeStats {
    let mut snapshot = XbxEngineMediaRuntimeStats::default();
    pipeline.write_runtime_stats(&mut snapshot);
    snapshot
}

fn apply_video_pipeline_runtime_stats(
    target: &mut XbxEngineMediaRuntimeStats,
    source: &XbxEngineMediaRuntimeStats,
) {
    target.latest_video_packet_arrival_time_ms = source.latest_video_packet_arrival_time_ms;
    target.latest_video_packet_sequence = source.latest_video_packet_sequence;
    target.inbound_video_packet_count_total = source.inbound_video_packet_count_total;
    target.inbound_video_packet_loss_estimate_total =
        source.inbound_video_packet_loss_estimate_total;
    target.inbound_video_loss_ratio_1s = source.inbound_video_loss_ratio_1s;
    target.inbound_video_loss_ratio_5s = source.inbound_video_loss_ratio_5s;
    target.inbound_video_jitter_ms = source.inbound_video_jitter_ms;
    target.video_nack_request_count_total = source.video_nack_request_count_total;
    target.video_nack_batch_count_total = source.video_nack_batch_count_total;
    target.video_nack_per_sec = source.video_nack_per_sec;
    target.video_pli_request_count_total = source.video_pli_request_count_total;
    target.video_pli_per_min = source.video_pli_per_min;
    target.video_pending_missing_packets = source.video_pending_missing_packets;
    target.video_loss_finalized_count_total = source.video_loss_finalized_count_total;
    target.video_loss_recovered_count_total = source.video_loss_recovered_count_total;
    target.video_loss_late_recovered_count_total = source.video_loss_late_recovered_count_total;
    target.video_nack_recovery_rtt_ms = source.video_nack_recovery_rtt_ms;
    target.video_rtt_ms = source.video_rtt_ms;
    target.video_remb_bps = source.video_remb_bps;
}

async fn request_video_packet_loss_indication(
    peer_connection: &Arc<RTCPeerConnection>,
    media_ssrc: u32,
) -> Result<(), webrtc::Error> {
    peer_connection
        .write_rtcp(&[Box::new(PictureLossIndication {
            sender_ssrc: FORCE_REMB_SENDER_SSRC,
            media_ssrc,
        })])
        .await
        .map(|_| ())
}

async fn request_transport_layer_nack(
    peer_connection: &Arc<RTCPeerConnection>,
    media_ssrc: u32,
    sequence_numbers: &[u16],
) -> Result<(), webrtc::Error> {
    if sequence_numbers.is_empty() {
        return Ok(());
    }
    peer_connection
        .write_rtcp(&[Box::new(TransportLayerNack {
            sender_ssrc: FORCE_REMB_SENDER_SSRC,
            media_ssrc,
            nacks: nack_pairs_from_sequence_numbers(sequence_numbers),
        })])
        .await
        .map(|_| ())
}

async fn request_receiver_estimated_maximum_bitrate(
    peer_connection: &Arc<RTCPeerConnection>,
    media_ssrc: u32,
    remb_bps: u32,
) -> Result<(), webrtc::Error> {
    peer_connection
        .write_rtcp(&[Box::new(ReceiverEstimatedMaximumBitrate {
            sender_ssrc: FORCE_REMB_SENDER_SSRC,
            bitrate: remb_bps as f32,
            ssrcs: vec![media_ssrc],
        })])
        .await
        .map(|_| ())
}

#[derive(Clone, Debug, Default)]
struct RttSampleSnapshot {
    selected_rtt_ms: Option<f64>,
    selected_source: Option<&'static str>,
    candidate_pair_current_rtt_ms: Option<f64>,
    candidate_pair_avg_rtt_ms: Option<f64>,
    remote_outbound_current_rtt_ms: Option<f64>,
    remote_outbound_avg_rtt_ms: Option<f64>,
    remote_inbound_current_rtt_ms: Option<f64>,
    remote_inbound_avg_rtt_ms: Option<f64>,
    candidate_pair_report_count: usize,
    candidate_pair_succeeded_count: usize,
    nominated_candidate_pair_count: usize,
    remote_outbound_video_report_count: usize,
    remote_outbound_target_ssrc_report_count: usize,
    remote_inbound_video_report_count: usize,
    remote_inbound_target_ssrc_report_count: usize,
    diagnostic_candidate_pair: Option<RttCandidatePairDiagnostic>,
}

#[derive(Clone, Debug)]
struct RttCandidatePairDiagnostic {
    id: String,
    local_candidate_id: String,
    remote_candidate_id: String,
    state: String,
    nominated: bool,
    current_round_trip_time_raw: f64,
    total_round_trip_time_raw: f64,
    responses_received_raw: u64,
}

#[derive(Clone, Copy, Debug)]
struct RttStreamCandidate {
    current_rtt_ms: Option<f64>,
    avg_rtt_ms: Option<f64>,
    score: u16,
}

async fn sample_peer_connection_rtt(
    peer_connection: &Arc<RTCPeerConnection>,
    target_video_ssrc: u32,
) -> RttSampleSnapshot {
    let stats = peer_connection.get_stats().await;
    let mut sample = RttSampleSnapshot::default();
    let mut selected_candidate_pair: Option<&ICECandidatePairStats> = None;
    let mut selected_candidate_pair_score = 0u8;
    let mut best_remote_outbound: Option<RttStreamCandidate> = None;
    let mut best_remote_inbound: Option<RttStreamCandidate> = None;

    for report in stats.reports.values() {
        match report {
            StatsReportType::CandidatePair(candidate_pair) => {
                sample.candidate_pair_report_count =
                    sample.candidate_pair_report_count.saturating_add(1);
                let is_succeeded =
                    format!("{:?}", candidate_pair.state).eq_ignore_ascii_case("succeeded");
                if is_succeeded {
                    sample.candidate_pair_succeeded_count =
                        sample.candidate_pair_succeeded_count.saturating_add(1);
                }
                if candidate_pair.nominated {
                    sample.nominated_candidate_pair_count =
                        sample.nominated_candidate_pair_count.saturating_add(1);
                }
                let current_rtt_ms = normalize_rtt_ms(candidate_pair.current_round_trip_time);
                let avg_rtt_ms = normalize_rtt_average_ms(
                    candidate_pair.total_round_trip_time,
                    candidate_pair.responses_received,
                );
                let has_rtt = current_rtt_ms.is_some() || avg_rtt_ms.is_some();
                let mut score = 0u8;
                if is_succeeded {
                    score = score.saturating_add(4);
                }
                if candidate_pair.nominated {
                    score = score.saturating_add(2);
                }
                if has_rtt {
                    score = score.saturating_add(1);
                }
                if candidate_pair.responses_received > 0 {
                    score = score.saturating_add(1);
                }
                if selected_candidate_pair.is_none() || score > selected_candidate_pair_score {
                    selected_candidate_pair_score = score;
                    selected_candidate_pair = Some(candidate_pair);
                }
            }
            StatsReportType::RemoteOutboundRTP(remote_outbound) => {
                if remote_outbound.kind != "video" {
                    continue;
                }
                sample.remote_outbound_video_report_count =
                    sample.remote_outbound_video_report_count.saturating_add(1);
                let is_target_ssrc = remote_outbound.ssrc == target_video_ssrc;
                if is_target_ssrc {
                    sample.remote_outbound_target_ssrc_report_count = sample
                        .remote_outbound_target_ssrc_report_count
                        .saturating_add(1);
                }
                let current_rtt_ms = remote_outbound.round_trip_time.and_then(normalize_rtt_ms);
                let avg_rtt_ms = normalize_rtt_average_ms(
                    remote_outbound.total_round_trip_time,
                    remote_outbound.round_trip_time_measurements,
                );
                let mut score = 0u16;
                if is_target_ssrc {
                    score = score.saturating_add(100);
                }
                if remote_outbound.round_trip_time_measurements > 0 {
                    score = score.saturating_add(20);
                }
                if remote_outbound.reports_sent > 0 {
                    score = score.saturating_add(5);
                }
                if current_rtt_ms.is_some() {
                    score = score.saturating_add(4);
                }
                if avg_rtt_ms.is_some() {
                    score = score.saturating_add(2);
                }
                let candidate = RttStreamCandidate {
                    current_rtt_ms,
                    avg_rtt_ms,
                    score,
                };
                if best_remote_outbound
                    .map(|current| candidate.score > current.score)
                    .unwrap_or(true)
                {
                    best_remote_outbound = Some(candidate);
                }
            }
            StatsReportType::RemoteInboundRTP(remote_inbound) => {
                if remote_inbound.kind != "video" {
                    continue;
                }
                sample.remote_inbound_video_report_count =
                    sample.remote_inbound_video_report_count.saturating_add(1);
                let is_target_ssrc = remote_inbound.ssrc == target_video_ssrc;
                if is_target_ssrc {
                    sample.remote_inbound_target_ssrc_report_count = sample
                        .remote_inbound_target_ssrc_report_count
                        .saturating_add(1);
                }
                let current_rtt_ms = remote_inbound.round_trip_time.and_then(normalize_rtt_ms);
                let avg_rtt_ms = normalize_rtt_average_ms(
                    remote_inbound.total_round_trip_time,
                    remote_inbound.round_trip_time_measurements,
                );
                let mut score = 0u16;
                if is_target_ssrc {
                    score = score.saturating_add(100);
                }
                if remote_inbound.round_trip_time_measurements > 0 {
                    score = score.saturating_add(20);
                }
                if current_rtt_ms.is_some() {
                    score = score.saturating_add(4);
                }
                if avg_rtt_ms.is_some() {
                    score = score.saturating_add(2);
                }
                let candidate = RttStreamCandidate {
                    current_rtt_ms,
                    avg_rtt_ms,
                    score,
                };
                if best_remote_inbound
                    .map(|current| candidate.score > current.score)
                    .unwrap_or(true)
                {
                    best_remote_inbound = Some(candidate);
                }
            }
            _ => {}
        }
    }
    if let Some(best) = best_remote_outbound {
        sample.remote_outbound_current_rtt_ms = best.current_rtt_ms;
        sample.remote_outbound_avg_rtt_ms = best.avg_rtt_ms;
    }
    if let Some(best) = best_remote_inbound {
        sample.remote_inbound_current_rtt_ms = best.current_rtt_ms;
        sample.remote_inbound_avg_rtt_ms = best.avg_rtt_ms;
    }
    if let Some(candidate_pair) = selected_candidate_pair {
        update_diagnostic_candidate_pair(&mut sample, candidate_pair);
        let normalized = normalize_rtt_ms(candidate_pair.current_round_trip_time);
        if normalized.is_some() {
            sample.candidate_pair_current_rtt_ms = normalized;
        }
        let average = normalize_rtt_average_ms(
            candidate_pair.total_round_trip_time,
            candidate_pair.responses_received,
        );
        if average.is_some() {
            sample.candidate_pair_avg_rtt_ms = average;
        }
    }

    let prioritized_sources: [(&str, Option<f64>); 6] = [
        (
            "candidate_pair_current",
            sample.candidate_pair_current_rtt_ms,
        ),
        ("candidate_pair_avg", sample.candidate_pair_avg_rtt_ms),
        (
            "remote_outbound_current",
            sample.remote_outbound_current_rtt_ms,
        ),
        ("remote_outbound_avg", sample.remote_outbound_avg_rtt_ms),
        (
            "remote_inbound_current",
            sample.remote_inbound_current_rtt_ms,
        ),
        ("remote_inbound_avg", sample.remote_inbound_avg_rtt_ms),
    ];
    for (source, value_ms) in prioritized_sources {
        if value_ms.is_some() {
            sample.selected_rtt_ms = value_ms;
            sample.selected_source = Some(source);
            break;
        }
    }
    sample
}

fn update_diagnostic_candidate_pair(
    sample: &mut RttSampleSnapshot,
    candidate_pair: &ICECandidatePairStats,
) {
    let next = RttCandidatePairDiagnostic {
        id: candidate_pair.id.clone(),
        local_candidate_id: candidate_pair.local_candidate_id.clone(),
        remote_candidate_id: candidate_pair.remote_candidate_id.clone(),
        state: format!("{:?}", candidate_pair.state),
        nominated: candidate_pair.nominated,
        current_round_trip_time_raw: candidate_pair.current_round_trip_time,
        total_round_trip_time_raw: candidate_pair.total_round_trip_time,
        responses_received_raw: candidate_pair.responses_received,
    };
    let should_replace = sample
        .diagnostic_candidate_pair
        .as_ref()
        .map(|current| !current.nominated && next.nominated)
        .unwrap_or(true);
    if should_replace {
        sample.diagnostic_candidate_pair = Some(next);
    }
}

fn maybe_log_rtt_diagnostics(
    config: &XbxEngineRttDiagnosticsRuntimeConfig,
    sample: &RttSampleSnapshot,
    now_ms: f64,
    last_logged_at_ms: &mut Option<f64>,
) {
    if !config.enabled {
        return;
    }
    let should_log = last_logged_at_ms
        .map(|last| now_ms - last >= config.log_interval_ms as f64)
        .unwrap_or(true);
    if !should_log {
        return;
    }
    *last_logged_at_ms = Some(now_ms);
    let candidate_pair = sample.diagnostic_candidate_pair.as_ref();
    eprintln!(
        "[xbxengine][webrtc-rs][rtt] selected={} source={} cpCurrentMs={} cpAvgMs={} roCurrentMs={} roAvgMs={} riCurrentMs={} riAvgMs={} cpReports={} cpSucceeded={} cpNominated={} roVideoReports={} roTargetSsrcReports={} riVideoReports={} riTargetSsrcReports={} cpId={} cpState={} cpLocal={} cpRemote={} cpRawCurrent={} cpRawTotal={} cpRawResponses={}",
        format_optional_ms(sample.selected_rtt_ms),
        sample.selected_source.unwrap_or("none"),
        format_optional_ms(sample.candidate_pair_current_rtt_ms),
        format_optional_ms(sample.candidate_pair_avg_rtt_ms),
        format_optional_ms(sample.remote_outbound_current_rtt_ms),
        format_optional_ms(sample.remote_outbound_avg_rtt_ms),
        format_optional_ms(sample.remote_inbound_current_rtt_ms),
        format_optional_ms(sample.remote_inbound_avg_rtt_ms),
        sample.candidate_pair_report_count,
        sample.candidate_pair_succeeded_count,
        sample.nominated_candidate_pair_count,
        sample.remote_outbound_video_report_count,
        sample.remote_outbound_target_ssrc_report_count,
        sample.remote_inbound_video_report_count,
        sample.remote_inbound_target_ssrc_report_count,
        candidate_pair.map(|value| value.id.as_str()).unwrap_or("none"),
        candidate_pair
            .map(|value| value.state.as_str())
            .unwrap_or("none"),
        candidate_pair
            .map(|value| value.local_candidate_id.as_str())
            .unwrap_or("none"),
        candidate_pair
            .map(|value| value.remote_candidate_id.as_str())
            .unwrap_or("none"),
        candidate_pair
            .map(|value| format_raw_number(value.current_round_trip_time_raw))
            .unwrap_or_else(|| "null".to_string()),
        candidate_pair
            .map(|value| format_raw_number(value.total_round_trip_time_raw))
            .unwrap_or_else(|| "null".to_string()),
        candidate_pair
            .map(|value| value.responses_received_raw.to_string())
            .unwrap_or_else(|| "null".to_string()),
    );
}

#[allow(clippy::too_many_arguments)]
fn maybe_log_control_diagnostics(
    config: &XbxEngineRttDiagnosticsRuntimeConfig,
    now_ms: f64,
    last_logged_at_ms: &mut Option<f64>,
    media_ssrc: u32,
    forced_remb_kbps: Option<u32>,
    adaptive_remb_enabled: bool,
    remb_bps: Option<u32>,
    loss1s_pct: Option<f64>,
    loss5s_pct: Option<f64>,
    jitter_ms: Option<f64>,
    rtt_ms: Option<f64>,
    rtt_source: Option<&str>,
) {
    if !config.enabled {
        return;
    }
    let should_log = last_logged_at_ms
        .map(|last| now_ms - last >= config.log_interval_ms as f64)
        .unwrap_or(true);
    if !should_log {
        return;
    }
    *last_logged_at_ms = Some(now_ms);
    eprintln!(
        "[xbxengine][webrtc-rs][control] ssrc={} forcedRembKbps={} adaptiveRemb={} rembBps={} loss1sPct={} loss5sPct={} jitterMs={} rttMs={} rttSource={}",
        media_ssrc,
        forced_remb_kbps
            .map(|value| value.to_string())
            .unwrap_or_else(|| "null".to_string()),
        adaptive_remb_enabled,
        remb_bps
            .map(|value| value.to_string())
            .unwrap_or_else(|| "null".to_string()),
        loss1s_pct
            .map(|value| format!("{value:.2}"))
            .unwrap_or_else(|| "null".to_string()),
        loss5s_pct
            .map(|value| format!("{value:.2}"))
            .unwrap_or_else(|| "null".to_string()),
        jitter_ms
            .map(|value| format!("{value:.2}"))
            .unwrap_or_else(|| "null".to_string()),
        rtt_ms
            .map(|value| format!("{value:.2}"))
            .unwrap_or_else(|| "null".to_string()),
        rtt_source.unwrap_or("null"),
    );
}

#[allow(clippy::too_many_arguments)]
fn maybe_log_track_throughput_diagnostics(
    config: &XbxEngineRttDiagnosticsRuntimeConfig,
    now_ms: f64,
    window_started_at_ms: &mut Option<f64>,
    window_packet_count: &mut u64,
    window_payload_bytes: &mut u64,
    kind: RTPCodecType,
    mime: &str,
    payload_type: u8,
    ssrc: u32,
    is_video_control_track: bool,
    mid: &str,
    track_id: &str,
) {
    if !config.enabled {
        return;
    }
    let Some(started_at_ms) = *window_started_at_ms else {
        return;
    };
    let elapsed_ms = now_ms - started_at_ms;
    if elapsed_ms < config.log_interval_ms as f64 {
        return;
    }
    let bitrate_kbps = if elapsed_ms > 0.0 {
        (*window_payload_bytes as f64 * 8.0) / elapsed_ms
    } else {
        0.0
    };
    let avg_payload_bytes = if *window_packet_count > 0 {
        *window_payload_bytes as f64 / *window_packet_count as f64
    } else {
        0.0
    };
    eprintln!(
        "[xbxengine][webrtc-rs][track] kind={} mime={} payloadType={} ssrc={} controlTrack={} mid={} trackId={} packets={} payloadBytes={} bitrateKbps={:.1} avgPayloadBytes={:.1}",
        kind,
        mime,
        payload_type,
        ssrc,
        is_video_control_track,
        mid,
        track_id,
        *window_packet_count,
        *window_payload_bytes,
        bitrate_kbps,
        avg_payload_bytes,
    );
    *window_started_at_ms = Some(now_ms);
    *window_packet_count = 0;
    *window_payload_bytes = 0;
}

fn format_optional_ms(value_ms: Option<f64>) -> String {
    match value_ms {
        Some(value) => format!("{value:.2}"),
        None => "null".to_string(),
    }
}

fn format_raw_number(value: f64) -> String {
    if value.is_finite() {
        format!("{value:.6}")
    } else {
        "NaN".to_string()
    }
}

fn normalize_rtt_ms(value: f64) -> Option<f64> {
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    // webrtc-rs 统计口径通常是秒，做一次归一避免把秒误当毫秒。
    if value <= 10.0 {
        Some(value * 1000.0)
    } else {
        Some(value)
    }
}

fn normalize_rtt_average_ms(total_round_trip_time: f64, measurements: u64) -> Option<f64> {
    if measurements == 0 || !total_round_trip_time.is_finite() || total_round_trip_time <= 0.0 {
        return None;
    }
    normalize_rtt_ms(total_round_trip_time / measurements as f64)
}

fn configure_peer_connection_offer_primitives(
    runtime: &Runtime,
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

        eprintln!(
            "[xbxengine][webrtc-rs] add transceiver kind=audio direction={} current_direction={} mid={}",
            audio.direction(),
            audio.current_direction(),
            audio.mid().as_deref().unwrap_or("")
        );
        eprintln!(
            "[xbxengine][webrtc-rs] add transceiver kind=video direction={} current_direction={} mid={}",
            video.direction(),
            video.current_direction(),
            video.mid().as_deref().unwrap_or("")
        );

        let transceivers = peer_connection.get_transceivers().await;
        for transceiver in transceivers {
            eprintln!(
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
    runtime: &Runtime,
    peer_connection: &Arc<RTCPeerConnection>,
    data_channels: &mut BTreeMap<String, Arc<RTCDataChannel>>,
    data_channel_state: Arc<Mutex<WebRtcRsDataChannelState>>,
) -> Result<(), XbxEngineRuntimeError> {
    for profile in STREAM_DATA_CHANNEL_PROFILES {
        let channel = runtime.block_on(async {
            peer_connection
                .create_data_channel(
                    profile.name,
                    Some(RTCDataChannelInit {
                        ordered: Some(profile.ordered),
                        protocol: Some(profile.protocol.to_string()),
                        ..Default::default()
                    }),
                )
                .await
                .map_err(map_webrtc_error(format!(
                    "createDataChannelFailed:{}",
                    profile.name
                )))
        })?;
        eprintln!(
            "[xbxengine][webrtc-rs] local data channel created label={} protocol={} ordered={}",
            channel.label(),
            channel.protocol(),
            profile.ordered
        );
        data_channels.insert(profile.name.to_string(), channel);
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

// 对齐 webrtc-direct 的基础策略：用 `b=AS` 注入带宽行，再补充 x-google bitrate 参数。
fn apply_offer_policy_contract(offer_sdp: &str) -> String {
    let profile = current_webrtc_rs_negotiation_profile();
    // b=AS 使用 kbps 语义，避免把 bps 误注入为异常大值。
    let with_video_bitrate = set_media_bitrate_as(offer_sdp, "video", profile.max_bitrate_kbps);
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
    let profile = current_webrtc_rs_negotiation_profile();
    let mut lines = offer_sdp
        .split("\r\n")
        .map(ToOwned::to_owned)
        .collect::<Vec<String>>();
    let mut section_start = 0usize;
    let max_frame_size = ((profile.width + 15) / 16) * ((profile.height + 15) / 16);

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
                    ("x-google-min-bitrate", profile.min_bitrate_kbps.to_string()),
                    (
                        "x-google-start-bitrate",
                        profile.target_bitrate_kbps.to_string(),
                    ),
                    ("x-google-max-bitrate", profile.max_bitrate_kbps.to_string()),
                    ("max-fs", max_frame_size.to_string()),
                    ("max-fr", profile.max_frame_rate.to_string()),
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
