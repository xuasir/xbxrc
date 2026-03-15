use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::runtime::Handle;
use tokio::sync::mpsc;
use webrtc::{
    api::{
        interceptor_registry::configure_rtcp_reports,
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
        rtp_codec::{
            RTCRtpCodecCapability, RTCRtpCodecParameters, RTCRtpHeaderExtensionCapability,
            RTPCodecType,
        },
        rtp_transceiver_direction::RTCRtpTransceiverDirection,
        RTCPFeedback, RTCRtpTransceiverInit, TYPE_RTCP_FB_CCM, TYPE_RTCP_FB_GOOG_REMB,
        TYPE_RTCP_FB_NACK, TYPE_RTCP_FB_TRANSPORT_CC,
    },
    stats::{ICECandidateStats, StatsReport},
};

use crate::{
    api::runtime::XbxEngineNegotiationRuntimeConfig,
    media::video::render::renderer::XbxRenderState,
    transport::adapter::{FrameSource, WebrtcVideoAdapter},
    transport::webrtc::audio_output::XbxRemoteAudioPlaybackSession,
    transport::webrtc::data_channel::{
        install_data_channel_contracts, request_decoder_reset_on_control_channel,
        request_video_keyframe_on_control_channel, XbxDataChannelState,
    },
    transport::webrtc::microphone::XbxMicrophoneSession,
    transport::webrtc::twcc_owned_receiver::OwnedTwccReceiverBuilder,
    XbxEngineMediaNegotiationRequest, XbxEngineMediaRuntimeStats, XbxEngineRuntimeError,
    XbxEngineVideoBweObservation, XbxEngineVideoTwccObservation, XbxEngineWebRtcRuntimeConfig,
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
const TRANSPORT_CC_URI: &str =
    "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01";

const VIDEO_CONTROL_WARMUP_MS: f64 = 1_000.0;

pub(crate) struct XbxTransportState {
    peer_connection: Option<Arc<RTCPeerConnection>>,
    data_channels: BTreeMap<String, Arc<RTCDataChannel>>,
    local_candidates: Arc<Mutex<Vec<XbxEngineIceCandidateDto>>>,
    local_ice_gathering_complete: Arc<Mutex<bool>>,
    microphone_session: Option<XbxMicrophoneSession>,
    audio_playback_session: Arc<Mutex<Option<XbxRemoteAudioPlaybackSession>>>,
    audio_volume_bits: Arc<AtomicU32>,
    runtime_stats: Option<Arc<Mutex<XbxEngineMediaRuntimeStats>>>,
    task_generation: Arc<AtomicU64>,
    pub(crate) frame_source_tx: Arc<Mutex<Option<mpsc::Sender<Box<dyn FrameSource>>>>>,
}

impl Default for XbxTransportState {
    fn default() -> Self {
        Self {
            peer_connection: None,
            data_channels: BTreeMap::new(),
            local_candidates: Arc::new(Mutex::new(Vec::new())),
            local_ice_gathering_complete: Arc::new(Mutex::new(false)),
            microphone_session: None,
            audio_playback_session: Arc::new(Mutex::new(None)),
            audio_volume_bits: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            runtime_stats: None,
            task_generation: Arc::new(AtomicU64::new(0)),
            frame_source_tx: Arc::new(Mutex::new(None)),
        }
    }
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
        self.set_local_ice_gathering_complete(false);
        self.data_channels.clear();
        if let Ok(mut stats) = runtime_stats.lock() {
            *stats = XbxEngineMediaRuntimeStats {
                transport_state: XbxEngineTransportStateDto::New,
                ..Default::default()
            };
        }
        self.runtime_stats = Some(runtime_stats.clone());
        let task_generation = self.task_generation.fetch_add(1, Ordering::SeqCst) + 1;

        let peer_connection = Arc::new(runtime.block_on(async {
            let mut media_engine = MediaEngine::default();
            media_engine
                .register_default_codecs()
                .map_err(map_webrtc_error("registerDefaultCodecsFailed"))?;
            // 保留 transport-cc 能力面，但把接收侧 feedback ownership 收到我们自己的实现。
            // NACK 只保留 responder；真正的缺包检测/重传请求在 adapter 路径按 upstream 算法实现。
            let mut interceptor_registry = configure_rtcp_reports(configure_owned_nack(
                Default::default(),
                &mut media_engine,
                std::time::Duration::from_millis(
                    webrtc_config.video_pipeline.nack_retry_interval_ms.max(30),
                ),
                runtime_stats.clone(),
            ));
            configure_owned_twcc_receiver(
                &mut interceptor_registry,
                &mut media_engine,
                std::time::Duration::from_millis(
                    webrtc_config
                        .video_pipeline
                        .feedback_interval_ms
                        .clamp(50, 100),
                ),
                runtime_stats.clone(),
            )
            .map_err(map_webrtc_error("configureOwnedTwccReceiverFailed"))?;
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
            self.local_ice_gathering_complete.clone(),
            runtime_stats.clone(),
            render_state,
            webrtc_config.clone(),
            self.frame_source_tx.clone(),
            self.audio_playback_session.clone(),
            self.audio_volume_bits.clone(),
            self.task_generation.clone(),
            task_generation,
        );
        configure_peer_connection_offer_primitives(runtime, &peer_connection)?;
        create_initial_data_channels(
            runtime,
            &peer_connection,
            &mut self.data_channels,
            data_channel_state,
            runtime_stats.clone(),
        )?;
        self.peer_connection = Some(peer_connection);
        Ok(())
    }

    pub(crate) fn create_offer(
        &self,
        runtime: &Handle,
        negotiation_config: &XbxEngineNegotiationRuntimeConfig,
    ) -> Result<String, XbxEngineRuntimeError> {
        let peer_connection = self.require_peer_connection()?;
        let (local_offer, patched_offer_sdp) = runtime.block_on(async {
            let offer = peer_connection
                .create_offer(None)
                .await
                .map_err(map_webrtc_error("createOfferFailed"))?;
            let patched_offer_sdp = apply_offer_policy_contract(&offer.sdp, negotiation_config);
            validate_local_offer_sdp(&patched_offer_sdp)?;
            // webrtc-rs 会校验 set_local_description 的 SDP 必须与 create_offer 产物一致，
            // 因此这里先使用原始 offer 建立本地状态，再把 patched SDP 单独作为上送文本返回。
            let local_offer = RTCSessionDescription::offer(offer.sdp.clone())
                .map_err(map_webrtc_error("buildLocalOfferFailed"))?;
            peer_connection
                .set_local_description(local_offer)
                .await
                .map_err(map_webrtc_error("setLocalDescriptionFailed"))?;
            let local_description = peer_connection
                .local_description()
                .await
                .ok_or_else(|| XbxEngineRuntimeError::new("localDescriptionMissing"))?;
            Ok::<_, XbxEngineRuntimeError>((local_description, patched_offer_sdp))
        })?;
        crate::xbx_log_info!(
            "[xbxengine][webrtc-rs] local offer created {}",
            summarize_sdp(&patched_offer_sdp)
        );
        crate::xbx_log_info!(
            "[xbxengine][webrtc-rs] local offer raw\n{}",
            patched_offer_sdp
        );
        let _ = local_offer;
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
            crate::xbx_log_info!("[xbxengine][webrtc-rs] remote answer raw\n{}", answer_sdp);

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

    pub(crate) fn add_remote_ice_candidates(
        &self,
        runtime: &Handle,
        remote_candidates: &[XbxEngineIceCandidateDto],
    ) -> Result<(), XbxEngineRuntimeError> {
        let peer_connection = self.require_peer_connection()?;
        runtime.block_on(async {
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

    pub(crate) fn local_ice_gathering_complete(&self) -> bool {
        self.local_ice_gathering_complete
            .lock()
            .ok()
            .map(|guard| *guard)
            .unwrap_or(false)
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

    pub(crate) fn set_audio_volume(&self, value: f32) {
        self.audio_volume_bits
            .store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub(crate) fn stop_peer_connection(&mut self, runtime: &Handle) {
        self.task_generation.fetch_add(1, Ordering::SeqCst);
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
        if let Ok(mut audio_session) = self.audio_playback_session.lock() {
            if let Some(session) = audio_session.take() {
                session.stop();
            }
        }
        if let Some(peer_connection) = self.peer_connection.take() {
            let _ = runtime.block_on(async { peer_connection.close().await });
        }
        self.data_channels.clear();
        if let Some(runtime_stats) = self.runtime_stats.as_ref() {
            if let Ok(mut stats) = runtime_stats.lock() {
                // stop 后立即清掉会污染后续 trace 的瞬态观测，避免 Closed 会话继续冒泡旧数据。
                stats.transport_state = XbxEngineTransportStateDto::Closed;
                stats.transport_path = None;
                stats.video_remb_bps = None;
                stats.video_rtt_ms = None;
                stats.video_rtt_source = None;
                stats.inbound_bitrate_kbps = None;
                stats.inbound_video_bitrate_kbps = None;
                stats.inbound_audio_bitrate_kbps = None;
                stats.inbound_audio_bytes_total = 0;
                stats.latest_video_bwe_observation = None;
                stats.latest_video_twcc_observation = None;
                stats.latest_video_packet_gap = None;
                stats.latest_video_nack_observation = None;
                stats.latest_video_escalation_observation = None;
                stats.latest_video_packet_arrival_time_ms = None;
                stats.latest_video_frame = None;
            }
        }
        // Warning: Do NOT take frame_source_tx here, rebuild_peer_connection calls stop_peer_connection
        // and we want the same tx to be used for the next connection.
    }

    fn clear_local_candidates(&self) {
        if let Ok(mut candidates) = self.local_candidates.lock() {
            candidates.clear();
        }
    }

    fn set_local_ice_gathering_complete(&self, complete: bool) {
        if let Ok(mut state) = self.local_ice_gathering_complete.lock() {
            *state = complete;
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
    let audio_playback_session_for_track = audio_playback_session.clone();
    let audio_volume_bits_for_track = audio_volume_bits.clone();

    peer_connection.on_track(Box::new(move |track, _, _transceiver| {
        let frame_source_tx = frame_source_tx.clone();
        let pc_captured = peer_connection_for_track.clone();
        let config_captured = webrtc_config_for_track.clone();
        let _stats_captured = runtime_stats_for_track.clone();
        let task_generation_for_track = task_generation.clone();
        let audio_playback_session = audio_playback_session_for_track.clone();
        let audio_volume_bits = audio_volume_bits_for_track.clone();

        Box::pin(async move {
            crate::xbx_log_info!("[xbxengine][webrtc-rs] ON_TRACK received: kind={} ssrc={} mime={}", track.kind(), track.ssrc(), track.codec().capability.mime_type);
            crate::xbx_log_debug!("[xbxengine][webrtc-rs] remote track kind={} mime={}", track.kind(), track.codec().capability.mime_type);

            let is_audio = track.kind() == webrtc::rtp_transceiver::rtp_codec::RTPCodecType::Audio;
            let is_video = track.kind() == webrtc::rtp_transceiver::rtp_codec::RTPCodecType::Video;
            let video_mime_type = track.codec().capability.mime_type.to_ascii_lowercase();
            let is_primary_video_track = is_video && video_mime_type == "video/h264";

            if is_primary_video_track {
                let jitter_buffer_size = config_captured.video_pipeline.jitter_buffer_max_packets;
                let idle_timeout = std::time::Duration::from_millis(config_captured.video_pipeline.idle_timeout_ms);

                crate::xbx_log_info!(
                    "[xbxengine][webrtc-rs] mounting video track with jitter_buffer={} idle_timeout={:?}",
                    jitter_buffer_size, idle_timeout
                );

                let adapter = WebrtcVideoAdapter::new(
                    track.clone(),
                    pc_captured.clone(),
                    _stats_captured.clone(),
                    jitter_buffer_size,
                    std::time::Duration::from_millis(
                        config_captured.video_pipeline.jitter_buffer_min_delay_ms,
                    ),
                    std::time::Duration::from_millis(
                        config_captured.video_pipeline.jitter_buffer_max_delay_ms,
                    ),
                    idle_timeout,
                    crate::transport::webrtc::nack_scheduler::NackSchedulerConfig {
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
                    let feedback_interval = std::time::Duration::from_millis(
                        config_captured.video_pipeline.feedback_interval_ms.max(50),
                    );
                    let mut interval = tokio::time::interval(feedback_interval);
                    let mut last_bytes_received = 0;
                    let mut last_packets_received = 0u64;
                    let mut last_video_sample_at_ms = now_ms_f64();
                    let mut last_loss_estimate_total = 0u64;
                    let mut last_loss_recovered_total = 0u64;
                    let mut last_loss_finalized_total = 0u64;
                    let mut tick_count = 0u64;
                    let mut bwe_observation_id = 0u64;
                    let mut last_sent_remb_kbps =
                        config_captured.forced_remb_kbps.unwrap_or(config_captured.remb_floor_kbps);
                    let mut hybrid_ramp_cooldown_ticks = 0u8;
                    loop {
                        interval.tick().await;
                        if task_generation_for_track.load(Ordering::SeqCst) != current_generation {
                            break;
                        }
                        tick_count += 1;
                        // 这里我们使用 pc 统一拉取 stats
                        let stats = pc_for_stats.get_stats().await;
                        let mut current_bytes = 0;
                        let mut packets_received = 0u64;
                        let mut packets_lost = 0i64;
                        let mut rtt = 0.0f64;
                        let mut rtt_source: Option<&'static str> = None;
                        let mut fraction_lost = 0.0f64;
                        let mut candidate_pair_rtt = 0.0f64;
                        let mut synthetic_loss_ratio = 0.0f64;

                        let mut report_counts = std::collections::HashMap::<String, usize>::new();
                        // ICE 层可用带宽（nominated pair 的 available_outgoing_bitrate，
                        // 基于 REMB 信令计算，反映网络容量上限）
                        let mut avail_bps = 0.0f64;
                        let mut avail_in_bps = 0.0f64;
                        let transport_path = resolve_transport_path(&stats);
                        let selected_candidate_pair = select_preferred_candidate_pair(&stats);
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
                                    if let Some(selected_pair) = selected_candidate_pair {
                                        if pair.id == selected_pair.id
                                            && pair.current_round_trip_time > 0.0
                                        {
                                            candidate_pair_rtt = pair.current_round_trip_time;
                                        }
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

                        if rtt <= 0.0 && candidate_pair_rtt > 0.0 {
                            rtt = candidate_pair_rtt;
                            rtt_source = Some("candidate-pair");
                        }

                        let sample_now_ms = now_ms_f64();
                        let elapsed_ms = (sample_now_ms - last_video_sample_at_ms).max(0.0);
                        let delta_bytes = current_bytes.saturating_sub(last_bytes_received);
                        last_bytes_received = current_bytes;
                        let delta_packets_received =
                            packets_received.saturating_sub(last_packets_received);
                        last_packets_received = packets_received;
                        let actual_kbps = (delta_bytes * 8) as f64 / elapsed_ms.max(1.0);
                        last_video_sample_at_ms = sample_now_ms;

                        if let Ok(shared) = _stats_captured.lock() {
                            let delta_loss_estimate = shared
                                .inbound_video_packet_loss_estimate_total
                                .saturating_sub(last_loss_estimate_total);
                            let delta_loss_recovered = shared
                                .video_loss_recovered_count_total
                                .saturating_sub(last_loss_recovered_total);
                            let delta_loss_finalized = shared
                                .video_loss_finalized_count_total
                                .saturating_sub(last_loss_finalized_total);
                            last_loss_estimate_total = shared.inbound_video_packet_loss_estimate_total;
                            last_loss_recovered_total = shared.video_loss_recovered_count_total;
                            last_loss_finalized_total = shared.video_loss_finalized_count_total;

                            let effective_loss_packets = delta_loss_finalized.max(
                                delta_loss_estimate.saturating_sub(delta_loss_recovered),
                            );
                            let loss_denominator =
                                delta_packets_received.saturating_add(effective_loss_packets);
                            if loss_denominator > 0 {
                                synthetic_loss_ratio =
                                    effective_loss_packets as f64 / loss_denominator as f64;
                            }
                            if fraction_lost <= 0.0 && synthetic_loss_ratio > 0.0 {
                                fraction_lost = synthetic_loss_ratio;
                                packets_lost = effective_loss_packets as i64;
                            }
                            if rtt <= 0.0 {
                                if let Some(nack_rtt_ms) = shared.video_nack_recovery_rtt_ms {
                                    rtt = nack_rtt_ms / 1000.0;
                                    rtt_source = Some("nack-recovery");
                                }
                            }
                        }

                        let observed_remb_kbps = if avail_bps > 0.0 {
                            Some((avail_bps / 1000.0).round().max(0.0) as u32)
                        } else {
                            None
                        };
                        let latest_twcc_observation = _stats_captured
                            .lock()
                            .ok()
                            .and_then(|shared| shared.latest_video_twcc_observation.clone());
                        let bwe_decision = resolve_target_remb_kbps(
                            &config_captured,
                            observed_remb_kbps,
                            actual_kbps,
                            fraction_lost,
                            transport_path.as_deref(),
                            latest_twcc_observation.as_ref(),
                            &mut last_sent_remb_kbps,
                            &mut hybrid_ramp_cooldown_ticks,
                        );
                        let target_remb_kbps = bwe_decision.target_kbps;
                        let observed_at_ms = now_ms_f64();

                        // 写回共享 stats
                        if let Ok(mut shared) = _stats_captured.lock() {
                            let twcc_feedback_interval_ms = shared
                                .latest_video_twcc_observation
                                .as_ref()
                                .and_then(|twcc| twcc.feedback_interval_ms);
                            let twcc_observed_packet_count = shared
                                .latest_video_twcc_observation
                                .as_ref()
                                .map(|twcc| twcc.observed_packet_count);
                            let twcc_covered_sequence_span = shared
                                .latest_video_twcc_observation
                                .as_ref()
                                .map(|twcc| twcc.covered_sequence_span);
                            let twcc_receive_bitrate_kbps = shared
                                .latest_video_twcc_observation
                                .as_ref()
                                .and_then(|twcc| twcc.receive_bitrate_kbps);
                            let twcc_delivery_ratio = shared
                                .latest_video_twcc_observation
                                .as_ref()
                                .map(|twcc| twcc.delivery_ratio);
                            let twcc_loss_ratio = shared
                                .latest_video_twcc_observation
                                .as_ref()
                                .map(|twcc| twcc.packet_loss_ratio);
                            bwe_observation_id = bwe_observation_id.saturating_add(1);
                            shared.video_remb_bps = Some(target_remb_kbps.saturating_mul(1000));
                            shared.inbound_video_bitrate_kbps = Some(actual_kbps.max(0.0));
                            shared.inbound_bitrate_kbps = Some(
                                actual_kbps.max(0.0)
                                    + shared.inbound_audio_bitrate_kbps.unwrap_or(0.0),
                            );
                            shared.video_rtt_ms = if rtt > 0.0 { Some(rtt * 1000.0) } else { None };
                            shared.video_rtt_source = rtt_source.map(str::to_string);
                            shared.inbound_video_loss_ratio_5s = fraction_lost;
                            shared.inbound_video_loss_ratio_1s = synthetic_loss_ratio.max(fraction_lost);
                            shared.transport_path = transport_path.clone();
                            shared.inbound_primary_video_bytes_total = current_bytes;
                            shared.inbound_video_bytes_total = current_bytes;
                            shared.inbound_bytes_total =
                                shared.inbound_video_bytes_total + shared.inbound_audio_bytes_total;
                            shared.latest_video_bwe_observation =
                                Some(XbxEngineVideoBweObservation {
                                    observation_id: bwe_observation_id,
                                    mode: config_captured.bwe_mode.clone(),
                                    decision_reason: bwe_decision.reason.clone(),
                                    target_remb_kbps,
                                    observed_remb_kbps,
                                    actual_video_bitrate_kbps: actual_kbps.max(0.0),
                                    loss_ratio: fraction_lost,
                                    rtt_ms: if rtt > 0.0 {
                                        Some(rtt * 1000.0)
                                    } else {
                                        None
                                    },
                                    transport_path: transport_path.clone(),
                                    twcc_feedback_interval_ms,
                                    twcc_observed_packet_count,
                                    twcc_covered_sequence_span,
                                    twcc_receive_bitrate_kbps,
                                    twcc_delivery_ratio,
                                    twcc_loss_ratio,
                                    observed_at_ms,
                                });
                        }

                        // 上层 BWE controller：根据模式输出 REMB target。
                        use webrtc::rtcp::payload_feedbacks::receiver_estimated_maximum_bitrate::*;
                        let remb = ReceiverEstimatedMaximumBitrate {
                            bitrate: (target_remb_kbps as f32) * 1000.0,
                            ssrcs: vec![stats_track.ssrc()],
                            ..Default::default()
                        };
                        let inject_result = pc_for_stats.write_rtcp(&[Box::new(remb)]).await;

                        let display_observed_kbps =
                            observed_remb_kbps.map(|kbps| kbps as f64).unwrap_or(0.0);
                        let avail_in_kbps = avail_in_bps / 1000.0;

                        // 定期打印注入状态确认
                        if tick_count % 30 == 0 {
                             if inject_result.is_ok() {
                                 crate::xbx_log_info!(
                                     "[xbxengine][BWE] target={}kbps mode={} observed={}kbps",
                                     target_remb_kbps,
                                     config_captured.bwe_mode,
                                     display_observed_kbps as u32
                                 );
                             } else {
                                 crate::xbx_log_warn!("[xbxengine][BWE] REMB injection failed: {:?}", inject_result.err());
                             }
                        }

                        crate::xbx_log_info!(
                            "[NetworkStats] Video: {:.0} Kbps | Target: {} Kbps | Observed: {:.0} Kbps (in:{:.0}) | Lost: {} | RTT: {:.1}ms | Reason: {}",
                            actual_kbps,
                            target_remb_kbps,
                            display_observed_kbps,
                            avail_in_kbps,
                            packets_lost,
                            rtt * 1000.0,
                            bwe_decision.reason
                        );
                    }
                });

            } else if is_audio {
                crate::xbx_log_info!(
                    "[xbxengine][webrtc-rs] mounting audio playback track mime={}",
                    track.codec().capability.mime_type
                );
                mount_remote_audio_track(
                    track.clone(),
                    _stats_captured.clone(),
                    audio_playback_session,
                    audio_volume_bits,
                );
            } else {
                // 其他轨道暂时只排空，避免缓冲堆积阻塞主轨。
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
    tokio::spawn(async move {
        let mut total_audio_bytes = 0u64;
        let mut last_audio_sample_bytes = 0u64;
        let mut last_audio_sample_at_ms = now_ms_f64();
        while let Ok((rtp, _)) = track.read_rtp().await {
            total_audio_bytes = total_audio_bytes.saturating_add(rtp.payload.len() as u64);
            let now_ms = now_ms_f64();
            let elapsed_ms = (now_ms - last_audio_sample_at_ms).max(0.0);
            if let Ok(mut shared) = runtime_stats.lock() {
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
            }
        }
    });
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

struct BweDecision {
    target_kbps: u32,
    reason: String,
}

struct TwccGccInput<'a> {
    observation: &'a XbxEngineVideoTwccObservation,
}

fn resolve_target_remb_kbps(
    config: &crate::XbxEngineWebRtcRuntimeConfig,
    observed_remb_kbps: Option<u32>,
    actual_kbps: f64,
    loss_ratio: f64,
    transport_path: Option<&str>,
    twcc_observation: Option<&XbxEngineVideoTwccObservation>,
    last_sent_remb_kbps: &mut u32,
    hybrid_ramp_cooldown_ticks: &mut u8,
) -> BweDecision {
    let floor_kbps = config.remb_floor_kbps.max(1);
    let ceiling_kbps = config.remb_ceiling_kbps.max(floor_kbps);
    let forced_kbps = config
        .forced_remb_kbps
        .unwrap_or(ceiling_kbps)
        .clamp(floor_kbps, ceiling_kbps);
    let observed_kbps = observed_remb_kbps
        .unwrap_or(forced_kbps)
        .clamp(floor_kbps, ceiling_kbps);
    let current_kbps = (*last_sent_remb_kbps).clamp(floor_kbps, ceiling_kbps);
    let actual_headroom_kbps =
        ((actual_kbps * 1.25).round() as u32).clamp(floor_kbps, ceiling_kbps);
    let twcc_input = twcc_observation.map(|observation| TwccGccInput { observation });

    let (next_kbps, reason) = match config.bwe_mode.as_str() {
        "twcc-gcc" => resolve_twcc_gcc_target(
            config,
            current_kbps,
            actual_headroom_kbps,
            transport_path,
            twcc_input.as_ref(),
            hybrid_ramp_cooldown_ticks,
        ),
        "observed-remb" => (observed_kbps, "observed-remb".to_string()),
        "hybrid" => {
            if let Some(twcc) = twcc_input.as_ref() {
                resolve_twcc_gcc_target(
                    config,
                    current_kbps,
                    actual_headroom_kbps,
                    transport_path,
                    Some(twcc),
                    hybrid_ramp_cooldown_ticks,
                )
            } else {
                let severe_loss = loss_ratio >= 0.08;
                let sustained_loss = loss_ratio >= 0.01;
                let mild_loss = loss_ratio >= 0.005;
                let bitrate_overrun = actual_kbps > (current_kbps as f64 * 1.1);

                if severe_loss || bitrate_overrun {
                    *hybrid_ramp_cooldown_ticks = 12;
                    (
                        ((current_kbps as f64) * (config.remb_ramp_down_factor as f64 / 1000.0))
                            .round()
                            .max(floor_kbps as f64) as u32,
                        if severe_loss {
                            "hybrid-severe-loss-backoff".to_string()
                        } else {
                            "hybrid-bitrate-overrun-backoff".to_string()
                        },
                    )
                } else if sustained_loss {
                    *hybrid_ramp_cooldown_ticks = 10;
                    (
                        current_kbps.min(actual_headroom_kbps).max(floor_kbps),
                        "hybrid-sustained-loss-cap".to_string(),
                    )
                } else if mild_loss {
                    *hybrid_ramp_cooldown_ticks = 6;
                    (
                        current_kbps
                            .min(actual_headroom_kbps.saturating_add(config.remb_ramp_up_step_kbps))
                            .max(floor_kbps),
                        "hybrid-mild-loss-hold".to_string(),
                    )
                } else if *hybrid_ramp_cooldown_ticks > 0 {
                    *hybrid_ramp_cooldown_ticks = hybrid_ramp_cooldown_ticks.saturating_sub(1);
                    (current_kbps, "hybrid-ramp-cooldown".to_string())
                } else {
                    let desired_kbps = observed_kbps.min(ceiling_kbps);
                    (
                        current_kbps
                            .saturating_add(config.remb_ramp_up_step_kbps)
                            .min(desired_kbps)
                            .max(floor_kbps),
                        if observed_remb_kbps.is_some() {
                            "hybrid-ramp-up-observed".to_string()
                        } else {
                            "hybrid-ramp-up-ceiling".to_string()
                        },
                    )
                }
            }
        }
        _ => (forced_kbps, "fixed-remb".to_string()),
    };

    let clamped_kbps = next_kbps.clamp(floor_kbps, ceiling_kbps);
    *last_sent_remb_kbps = clamped_kbps;
    BweDecision {
        target_kbps: clamped_kbps,
        reason,
    }
}

fn resolve_twcc_gcc_target(
    config: &crate::XbxEngineWebRtcRuntimeConfig,
    current_kbps: u32,
    actual_headroom_kbps: u32,
    transport_path: Option<&str>,
    twcc_input: Option<&TwccGccInput<'_>>,
    ramp_cooldown_ticks: &mut u8,
) -> (u32, String) {
    let floor_kbps = config.remb_floor_kbps.max(1);
    let ceiling_kbps = config.remb_ceiling_kbps.max(floor_kbps);
    let direct_path = transport_path
        .map(|path| path.to_ascii_lowercase().starts_with("direct"))
        .unwrap_or(false);
    let preferred_gaming_floor_kbps = if direct_path {
        floor_kbps.max(30_000)
    } else {
        floor_kbps
    };
    let ramp_up_step_kbps = if direct_path {
        config.remb_ramp_up_step_kbps.saturating_mul(2)
    } else {
        config.remb_ramp_up_step_kbps
    };
    let Some(twcc_input) = twcc_input else {
        return (
            current_kbps.max(preferred_gaming_floor_kbps),
            if direct_path {
                "twcc-gcc-direct-await-feedback".to_string()
            } else {
                "twcc-gcc-await-feedback".to_string()
            },
        );
    };

    let twcc = twcc_input.observation;
    let stable_feedback = twcc.feedback_interval_ms.unwrap_or(0.0)
        <= if direct_path { 300.0 } else { 200.0 }
        && twcc.observed_packet_count >= if direct_path { 6 } else { 12 }
        && twcc.covered_sequence_span >= twcc.observed_packet_count;
    let receive_bitrate_kbps = twcc
        .receive_bitrate_kbps
        .unwrap_or(actual_headroom_kbps as f64)
        .clamp(floor_kbps as f64, ceiling_kbps as f64) as u32;
    let receive_headroom_kbps = ((receive_bitrate_kbps as f64)
        * if direct_path { 1.55 } else { 1.08 })
    .round()
    .clamp(floor_kbps as f64, ceiling_kbps as f64) as u32;

    if !stable_feedback {
        return (
            if direct_path {
                current_kbps.max(preferred_gaming_floor_kbps)
            } else {
                current_kbps.min(receive_headroom_kbps.max(floor_kbps))
            },
            if direct_path {
                "twcc-gcc-direct-unstable-hold".to_string()
            } else {
                "twcc-gcc-unstable-feedback-hold".to_string()
            },
        );
    }

    let severe_loss_threshold = if direct_path { 0.30 } else { 0.12 };
    let severe_delivery_threshold = if direct_path { 0.62 } else { 0.82 };
    let congestion_loss_threshold = if direct_path { 0.15 } else { 0.05 };
    let congestion_delivery_threshold = if direct_path { 0.80 } else { 0.92 };
    let mild_loss_threshold = if direct_path { 0.07 } else { 0.02 };
    let mild_delivery_threshold = if direct_path { 0.90 } else { 0.97 };

    if twcc.packet_loss_ratio >= severe_loss_threshold
        || twcc.delivery_ratio <= severe_delivery_threshold
    {
        *ramp_cooldown_ticks = if direct_path { 6 } else { 12 };
        let backoff_kbps = ((current_kbps as f64) * (config.remb_ramp_down_factor as f64 / 1000.0))
            .round()
            .max(floor_kbps as f64) as u32;
        return (
            backoff_kbps.min(receive_headroom_kbps.max(floor_kbps)),
            if direct_path {
                "twcc-gcc-direct-severe-backoff".to_string()
            } else {
                "twcc-gcc-severe-backoff".to_string()
            },
        );
    }

    if twcc.packet_loss_ratio >= congestion_loss_threshold
        || twcc.delivery_ratio <= congestion_delivery_threshold
    {
        *ramp_cooldown_ticks = if direct_path { 4 } else { 8 };
        let direct_cap_kbps = receive_headroom_kbps
            .saturating_add(ramp_up_step_kbps)
            .max(preferred_gaming_floor_kbps);
        return (
            if direct_path {
                current_kbps.min(direct_cap_kbps)
            } else {
                current_kbps.min(receive_headroom_kbps.max(floor_kbps))
            },
            if direct_path {
                "twcc-gcc-direct-congestion-cap".to_string()
            } else {
                "twcc-gcc-congestion-cap".to_string()
            },
        );
    }

    if twcc.packet_loss_ratio >= mild_loss_threshold
        || twcc.delivery_ratio <= mild_delivery_threshold
    {
        *ramp_cooldown_ticks = if direct_path { 2 } else { 4 };
        return (
            if direct_path {
                current_kbps.max(preferred_gaming_floor_kbps)
            } else {
                current_kbps
                    .min(receive_headroom_kbps.saturating_add(ramp_up_step_kbps))
                    .max(floor_kbps)
            },
            if direct_path {
                "twcc-gcc-direct-mild-hold".to_string()
            } else {
                "twcc-gcc-mild-hold".to_string()
            },
        );
    }

    if *ramp_cooldown_ticks > 0 {
        *ramp_cooldown_ticks = ramp_cooldown_ticks.saturating_sub(1);
        return (
            current_kbps.max(preferred_gaming_floor_kbps),
            if direct_path {
                "twcc-gcc-direct-ramp-cooldown".to_string()
            } else {
                "twcc-gcc-ramp-cooldown".to_string()
            },
        );
    }

    let desired_kbps = receive_headroom_kbps
        .max(actual_headroom_kbps)
        .max(preferred_gaming_floor_kbps)
        .clamp(floor_kbps, ceiling_kbps);
    (
        current_kbps
            .saturating_add(ramp_up_step_kbps)
            .min(desired_kbps)
            .max(floor_kbps),
        if direct_path {
            "twcc-gcc-direct-ramp-up".to_string()
        } else {
            "twcc-gcc-ramp-up".to_string()
        },
    )
}

fn configure_owned_nack(
    mut registry: interceptor::registry::Registry,
    media_engine: &mut MediaEngine,
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

fn configure_owned_twcc_receiver(
    registry: &mut interceptor::registry::Registry,
    media_engine: &mut MediaEngine,
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
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
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

    install_data_channel_contracts(data_channels, data_channel_state, runtime_stats)?;
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

// SDP policy 只负责把 runtime negotiation 配置投影到上送服务端的文本 offer。
fn apply_offer_policy_contract(
    offer_sdp: &str,
    negotiation_config: &XbxEngineNegotiationRuntimeConfig,
) -> String {
    let with_video_bitrate = set_media_bitrate_as(
        offer_sdp,
        "video",
        negotiation_config.video_bitrate_kbps.max(1),
    );
    let with_audio_bitrate = set_media_bitrate_as(
        &with_video_bitrate,
        "audio",
        negotiation_config.audio_bitrate_kbps.max(1),
    );
    let with_audio_layout = if negotiation_config.force_mono_audio {
        with_audio_bitrate
    } else {
        with_audio_bitrate.replace("useinbandfec=1", "useinbandfec=1; stereo=1")
    };
    let with_video_profile = reorder_h264_payload_types_by_profile(
        &with_audio_layout,
        &negotiation_config.offer_profile,
    );
    patch_video_fmtp_constraints(&with_video_profile, negotiation_config)
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

fn patch_video_fmtp_constraints(
    offer_sdp: &str,
    negotiation_config: &XbxEngineNegotiationRuntimeConfig,
) -> String {
    let mut lines = offer_sdp
        .split("\r\n")
        .map(ToOwned::to_owned)
        .collect::<Vec<String>>();
    let mut section_start = 0usize;
    let tier = resolve_offer_video_constraint_tier(negotiation_config);

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
                    ("x-google-min-bitrate", tier.min_bitrate_kbps.to_string()),
                    (
                        "x-google-start-bitrate",
                        tier.start_bitrate_kbps.to_string(),
                    ),
                    ("x-google-max-bitrate", tier.max_bitrate_kbps.to_string()),
                    ("max-fs", tier.max_frame_size.to_string()),
                    ("max-fr", "60".to_string()),
                ],
            );
        }

        section_start = section_end;
    }

    lines.join("\r\n")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OfferVideoConstraintTier {
    max_frame_size: u32,
    min_bitrate_kbps: u32,
    start_bitrate_kbps: u32,
    max_bitrate_kbps: u32,
}

fn resolve_offer_video_constraint_tier(
    negotiation_config: &XbxEngineNegotiationRuntimeConfig,
) -> OfferVideoConstraintTier {
    let width = negotiation_config.target_resolution_width.max(16);
    let height = negotiation_config.target_resolution_height.max(16);
    let max_frame_size = width.div_ceil(16).saturating_mul(height.div_ceil(16));
    let configured_max_bitrate_kbps = negotiation_config.video_bitrate_kbps.max(1);

    // 这里沿用 browser runtime / better-xcloud 的分档思路：
    // 720p 偏保守，1080p 中档，1440p+ 提前抬高 start bitrate，避免首屏长期糊帧。
    if height <= 720 {
        return OfferVideoConstraintTier {
            max_frame_size,
            min_bitrate_kbps: 3_000,
            start_bitrate_kbps: configured_max_bitrate_kbps.min(10_000),
            max_bitrate_kbps: configured_max_bitrate_kbps,
        };
    }

    if height > 1080 || width > 1920 {
        return OfferVideoConstraintTier {
            max_frame_size,
            min_bitrate_kbps: 8_000,
            start_bitrate_kbps: configured_max_bitrate_kbps.min(35_000),
            max_bitrate_kbps: configured_max_bitrate_kbps,
        };
    }

    OfferVideoConstraintTier {
        max_frame_size,
        min_bitrate_kbps: 5_000,
        start_bitrate_kbps: configured_max_bitrate_kbps.min(20_000),
        max_bitrate_kbps: configured_max_bitrate_kbps,
    }
}

fn reorder_h264_payload_types_by_profile(offer_sdp: &str, preferred_profile: &str) -> String {
    if preferred_profile.is_empty() {
        return offer_sdp.to_string();
    }

    let mut lines = offer_sdp
        .split("\r\n")
        .map(ToOwned::to_owned)
        .collect::<Vec<String>>();
    let mut section_start = 0usize;

    while section_start < lines.len() {
        if !lines[section_start].starts_with("m=video ") {
            section_start += 1;
            continue;
        }

        let mut section_end = section_start + 1;
        while section_end < lines.len() && !lines[section_end].starts_with("m=") {
            section_end += 1;
        }

        let preferred_payload_types = collect_h264_preferred_payload_types(
            &lines[section_start..section_end],
            preferred_profile,
        );
        if preferred_payload_types.is_empty() {
            section_start = section_end;
            continue;
        }

        let mut parts = lines[section_start]
            .split_whitespace()
            .map(ToOwned::to_owned)
            .collect::<Vec<String>>();
        if parts.len() > 3 {
            let reordered = preferred_payload_types
                .iter()
                .cloned()
                .chain(
                    parts[3..]
                        .iter()
                        .filter(|payload| !preferred_payload_types.contains(*payload))
                        .cloned(),
                )
                .collect::<Vec<String>>();
            parts.truncate(3);
            parts.extend(reordered);
            lines[section_start] = parts.join(" ");
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

fn collect_h264_preferred_payload_types(
    video_section_lines: &[String],
    preferred_profile: &str,
) -> Vec<String> {
    let h264_payload_types = collect_h264_payload_types(video_section_lines);
    let normalized_profile = preferred_profile.to_ascii_lowercase();
    let mut preferred_payload_types = Vec::new();

    for line in video_section_lines.iter().skip(1) {
        let Some(payload_type) = extract_fmtp_payload_type(line) else {
            continue;
        };
        if !h264_payload_types.contains(payload_type) {
            continue;
        }
        let normalized_line = line.to_ascii_lowercase();
        if normalized_line.contains(&format!("profile-level-id={normalized_profile}")) {
            preferred_payload_types.push(payload_type.to_string());
        }
    }

    preferred_payload_types
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

#[cfg(test)]
mod tests {
    use super::{
        apply_offer_policy_contract, resolve_offer_video_constraint_tier, resolve_target_remb_kbps,
        BweDecision, XbxEngineNegotiationRuntimeConfig,
    };
    use crate::XbxEngineWebRtcRuntimeConfig;

    fn sample_offer_sdp() -> String {
        [
            "v=0",
            "o=- 0 0 IN IP4 127.0.0.1",
            "s=-",
            "t=0 0",
            "m=audio 9 UDP/TLS/RTP/SAVPF 111",
            "c=IN IP4 0.0.0.0",
            "a=rtpmap:111 opus/48000/2",
            "a=fmtp:111 minptime=10;useinbandfec=1",
            "a=rtcp-fb:111 transport-cc",
            "a=extmap:1 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01",
            "m=video 9 UDP/TLS/RTP/SAVPF 102 104 106",
            "c=IN IP4 0.0.0.0",
            "a=rtpmap:102 H264/90000",
            "a=fmtp:102 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42001f",
            "a=rtcp-fb:102 transport-cc",
            "a=rtpmap:104 H264/90000",
            "a=fmtp:104 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f",
            "a=rtpmap:106 H264/90000",
            "a=fmtp:106 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=4d0032",
            "a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01",
            "m=application 9 UDP/DTLS/SCTP webrtc-datachannel",
        ]
        .join("\r\n")
    }

    #[test]
    fn resolve_video_constraint_tier_matches_browser_720p_profile() {
        let tier = resolve_offer_video_constraint_tier(&XbxEngineNegotiationRuntimeConfig {
            target_resolution_width: 1280,
            target_resolution_height: 720,
            video_bitrate_kbps: 18_000,
            ..Default::default()
        });

        assert_eq!(tier.min_bitrate_kbps, 3_000);
        assert_eq!(tier.start_bitrate_kbps, 10_000);
        assert_eq!(tier.max_bitrate_kbps, 18_000);
        assert_eq!(tier.max_frame_size, 3_600);
    }

    #[test]
    fn resolve_video_constraint_tier_matches_browser_1440p_profile() {
        let tier = resolve_offer_video_constraint_tier(&XbxEngineNegotiationRuntimeConfig {
            target_resolution_width: 2560,
            target_resolution_height: 1440,
            video_bitrate_kbps: 60_000,
            ..Default::default()
        });

        assert_eq!(tier.min_bitrate_kbps, 8_000);
        assert_eq!(tier.start_bitrate_kbps, 35_000);
        assert_eq!(tier.max_bitrate_kbps, 60_000);
        assert_eq!(tier.max_frame_size, 14_400);
    }

    #[test]
    fn apply_offer_policy_contract_uses_better_xcloud_style_profile_and_bitrate_tier() {
        let patched = apply_offer_policy_contract(
            &sample_offer_sdp(),
            &XbxEngineNegotiationRuntimeConfig {
                target_resolution_width: 1920,
                target_resolution_height: 1080,
                video_bitrate_kbps: 40_000,
                audio_bitrate_kbps: 192,
                force_mono_audio: false,
                offer_profile: "42e".to_string(),
            },
        );

        assert!(patched.contains("m=audio 9 UDP/TLS/RTP/SAVPF 111\r\nb=AS:192"));
        assert!(patched.contains("useinbandfec=1; stereo=1"));
        assert!(patched.contains("m=video 9 UDP/TLS/RTP/SAVPF 104 102 106"));
        assert!(patched.contains("x-google-min-bitrate=5000"));
        assert!(patched.contains("x-google-start-bitrate=20000"));
        assert!(patched.contains("x-google-max-bitrate=40000"));
        assert!(patched.contains("max-fs=8160"));
        assert!(patched.contains("max-fr=60"));
        assert!(patched.contains("a=rtcp-fb:111 transport-cc"));
        assert!(patched.contains("a=rtcp-fb:102 transport-cc"));
        assert!(patched.contains("draft-holmer-rmcat-transport-wide-cc-extensions-01"));
    }

    #[test]
    fn hybrid_bwe_caps_to_actual_headroom_when_loss_is_sustained() {
        let config = XbxEngineWebRtcRuntimeConfig {
            bwe_mode: "hybrid".to_string(),
            forced_remb_kbps: Some(100_000),
            remb_floor_kbps: 12_000,
            remb_ceiling_kbps: 100_000,
            remb_ramp_up_step_kbps: 4_000,
            remb_ramp_down_factor: 700,
            ..Default::default()
        };
        let mut last_sent_remb_kbps = 100_000;

        let BweDecision {
            target_kbps,
            reason,
        } = resolve_target_remb_kbps(
            &config,
            None,
            4_600.0,
            0.012,
            None,
            &mut last_sent_remb_kbps,
            &mut 0,
        );

        assert_eq!(target_kbps, 12_000);
        assert_eq!(reason, "hybrid-sustained-loss-cap");
        assert_eq!(last_sent_remb_kbps, 12_000);
    }

    #[test]
    fn hybrid_bwe_uses_multiplicative_backoff_for_severe_loss() {
        let config = XbxEngineWebRtcRuntimeConfig {
            bwe_mode: "hybrid".to_string(),
            forced_remb_kbps: Some(80_000),
            remb_floor_kbps: 12_000,
            remb_ceiling_kbps: 100_000,
            remb_ramp_up_step_kbps: 4_000,
            remb_ramp_down_factor: 700,
            ..Default::default()
        };
        let mut last_sent_remb_kbps = 50_000;

        let decision = resolve_target_remb_kbps(
            &config,
            None,
            9_000.0,
            0.12,
            None,
            &mut last_sent_remb_kbps,
            &mut 0,
        );

        assert_eq!(decision.target_kbps, 35_000);
        assert_eq!(decision.reason, "hybrid-severe-loss-backoff");
    }

    #[test]
    fn hybrid_bwe_holds_during_post_loss_cooldown() {
        let config = XbxEngineWebRtcRuntimeConfig {
            bwe_mode: "hybrid".to_string(),
            forced_remb_kbps: Some(100_000),
            remb_floor_kbps: 12_000,
            remb_ceiling_kbps: 100_000,
            remb_ramp_up_step_kbps: 4_000,
            remb_ramp_down_factor: 700,
            ..Default::default()
        };
        let mut last_sent_remb_kbps = 16_000;
        let mut cooldown_ticks = 3;

        let decision = resolve_target_remb_kbps(
            &config,
            None,
            5_200.0,
            0.0,
            None,
            &mut last_sent_remb_kbps,
            &mut cooldown_ticks,
        );

        assert_eq!(decision.target_kbps, 16_000);
        assert_eq!(decision.reason, "hybrid-ramp-cooldown");
        assert_eq!(cooldown_ticks, 2);
    }

    #[test]
    fn twcc_gcc_direct_path_prefers_higher_gaming_target() {
        let config = XbxEngineWebRtcRuntimeConfig {
            bwe_mode: "twcc-gcc".to_string(),
            forced_remb_kbps: Some(150_000),
            remb_floor_kbps: 25_000,
            remb_ceiling_kbps: 150_000,
            remb_ramp_up_step_kbps: 12_000,
            remb_ramp_down_factor: 900,
            ..Default::default()
        };
        let twcc = crate::XbxEngineVideoTwccObservation {
            observation_id: 1,
            feedback_packet_count: 1,
            covered_sequence_start: 100,
            covered_sequence_end: 239,
            covered_sequence_span: 140,
            observed_packet_count: 140,
            observed_byte_count: 150_000,
            feedback_interval_ms: Some(100.0),
            arrival_span_ms: Some(95.0),
            receive_bitrate_kbps: Some(10_500.0),
            delivery_ratio: 1.0,
            packet_loss_ratio: 0.0,
            observed_at_ms: 1_000.0,
        };
        let mut last_sent_remb_kbps = 25_000;
        let mut cooldown_ticks = 0;

        let decision = resolve_target_remb_kbps(
            &config,
            None,
            9_800.0,
            0.0,
            Some("Direct (host->host)"),
            Some(&twcc),
            &mut last_sent_remb_kbps,
            &mut cooldown_ticks,
        );

        assert_eq!(decision.reason, "twcc-gcc-direct-ramp-up");
        assert_eq!(decision.target_kbps, 30_000);
    }

    #[test]
    fn twcc_gcc_direct_path_keeps_high_target_under_mild_loss() {
        let config = XbxEngineWebRtcRuntimeConfig {
            bwe_mode: "twcc-gcc".to_string(),
            forced_remb_kbps: Some(150_000),
            remb_floor_kbps: 25_000,
            remb_ceiling_kbps: 150_000,
            remb_ramp_up_step_kbps: 12_000,
            remb_ramp_down_factor: 900,
            ..Default::default()
        };
        let twcc = crate::XbxEngineVideoTwccObservation {
            observation_id: 2,
            feedback_packet_count: 1,
            covered_sequence_start: 1_000,
            covered_sequence_end: 1_179,
            covered_sequence_span: 180,
            observed_packet_count: 168,
            observed_byte_count: 240_000,
            feedback_interval_ms: Some(102.0),
            arrival_span_ms: Some(98.0),
            receive_bitrate_kbps: Some(18_500.0),
            delivery_ratio: 0.93,
            packet_loss_ratio: 0.03,
            observed_at_ms: 5_000.0,
        };
        let mut last_sent_remb_kbps = 52_000;
        let mut cooldown_ticks = 0;

        let decision = resolve_target_remb_kbps(
            &config,
            None,
            18_000.0,
            0.0,
            Some("Direct (host->host)"),
            Some(&twcc),
            &mut last_sent_remb_kbps,
            &mut cooldown_ticks,
        );

        assert_eq!(decision.reason, "twcc-gcc-direct-mild-hold");
        assert_eq!(decision.target_kbps, 52_000);
    }
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

fn resolve_transport_path(stats: &StatsReport) -> Option<String> {
    let selected_pair = select_preferred_candidate_pair(stats);
    let mut local_candidates = std::collections::HashMap::<&str, &ICECandidateStats>::new();
    let mut remote_candidates = std::collections::HashMap::<&str, &ICECandidateStats>::new();

    for report in stats.reports.values() {
        match report {
            webrtc::stats::StatsReportType::LocalCandidate(candidate) => {
                local_candidates.insert(candidate.id.as_str(), candidate);
            }
            webrtc::stats::StatsReportType::RemoteCandidate(candidate) => {
                remote_candidates.insert(candidate.id.as_str(), candidate);
            }
            _ => {}
        }
    }

    let pair = selected_pair?;
    let local_candidate = local_candidates.get(pair.local_candidate_id.as_str())?;
    let remote_candidate = remote_candidates.get(pair.remote_candidate_id.as_str())?;
    let local_type = normalize_candidate_type(&local_candidate.candidate_type);
    let remote_type = normalize_candidate_type(&remote_candidate.candidate_type);
    let path_kind = if local_type == "relay" || remote_type == "relay" {
        "Relay"
    } else {
        "Direct"
    };
    Some(format!("{path_kind} ({local_type}->{remote_type})"))
}

fn select_preferred_candidate_pair(
    stats: &StatsReport,
) -> Option<&webrtc::stats::ICECandidatePairStats> {
    let mut nominated_pair: Option<&webrtc::stats::ICECandidatePairStats> = None;
    let mut active_pair: Option<&webrtc::stats::ICECandidatePairStats> = None;

    for report in stats.reports.values() {
        let webrtc::stats::StatsReportType::CandidatePair(pair) = report else {
            continue;
        };
        if pair.nominated {
            nominated_pair = Some(pair);
            break;
        }
        if active_pair.is_none()
            && (pair.available_outgoing_bitrate > 0.0
                || pair.available_incoming_bitrate > 0.0
                || pair.current_round_trip_time > 0.0)
        {
            active_pair = Some(pair);
        }
    }

    nominated_pair.or(active_pair)
}

fn normalize_candidate_type(candidate_type: &impl std::fmt::Debug) -> String {
    match format!("{candidate_type:?}").to_ascii_lowercase().as_str() {
        "host" => "host".to_string(),
        "serverreflexive" => "srflx".to_string(),
        "peerreflexive" => "prflx".to_string(),
        "relay" => "relay".to_string(),
        _ => "unknown".to_string(),
    }
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
