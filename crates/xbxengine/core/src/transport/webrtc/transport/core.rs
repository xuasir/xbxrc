use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::runtime::Handle;
use tokio::sync::mpsc;
use webrtc::{
    api::{interceptor_registry::configure_rtcp_reports, media_engine::MediaEngine, APIBuilder},
    data_channel::RTCDataChannel,
    ice_transport::ice_candidate::RTCIceCandidateInit,
    peer_connection::{sdp::session_description::RTCSessionDescription, RTCPeerConnection},
};

use super::{
    build_rtc_configuration, configure_owned_nack, configure_owned_twcc_receiver,
    configure_peer_connection_offer_primitives, create_initial_data_channels,
    install_peer_connection_callbacks, map_webrtc_error, normalize_remote_ice_candidate,
};
use crate::{
    api::runtime::XbxEngineNegotiationRuntimeConfig,
    media::video::render::renderer::XbxRenderState,
    transport::adapter::FrameSource,
    transport::webrtc::audio_output::XbxRemoteAudioPlaybackSession,
    transport::webrtc::data_channel::{
        request_decoder_reset_on_control_channel, request_video_keyframe_on_control_channel,
        XbxDataChannelState,
    },
    transport::webrtc::microphone::XbxMicrophoneSession,
    transport::webrtc::sdp_policy::{
        apply_offer_policy_contract, summarize_sdp, validate_local_offer_sdp,
    },
    XbxEngineMediaNegotiationRequest, XbxEngineMediaRuntimeStats, XbxEngineRuntimeError,
    XbxEngineWebRtcRuntimeConfig,
};
use xbxengine_protocol::{
    XbxEngineIceCandidateDto, XbxEngineTargetTypeDto, XbxEngineTransportStateDto,
};

pub(crate) struct XbxTransportState {
    peer_connection: Option<Arc<RTCPeerConnection>>,
    data_channels: BTreeMap<String, Arc<RTCDataChannel>>,
    local_candidates: Arc<Mutex<Vec<XbxEngineIceCandidateDto>>>,
    local_ice_gathering_complete: Arc<Mutex<bool>>,
    microphone_session: Option<XbxMicrophoneSession>,
    audio_playback_session: Arc<Mutex<Option<XbxRemoteAudioPlaybackSession>>>,
    audio_volume_bits: Arc<AtomicU32>,
    runtime_stats: Option<Arc<Mutex<XbxEngineMediaRuntimeStats>>>,
    data_channel_state: Option<Arc<Mutex<XbxDataChannelState>>>,
    session_target_type: Option<XbxEngineTargetTypeDto>,
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
            data_channel_state: None,
            session_target_type: None,
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
                session_target_type: Some(request.session.target_type.clone()),
                transport_state: XbxEngineTransportStateDto::New,
                ..Default::default()
            };
        }
        self.runtime_stats = Some(runtime_stats.clone());
        self.data_channel_state = Some(data_channel_state.clone());
        self.session_target_type = Some(request.session.target_type.clone());
        let task_generation = self.task_generation.fetch_add(1, Ordering::SeqCst) + 1;

        let peer_connection = Arc::new(runtime.block_on(async {
            let mut media_engine = MediaEngine::default();
            media_engine
                .register_default_codecs()
                .map_err(map_webrtc_error("registerDefaultCodecsFailed"))?;
            super::setup::register_owned_h264_codecs(&mut media_engine)
                .map_err(map_webrtc_error("registerOwnedH264CodecsFailed"))?;
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
            let patched_offer_sdp = apply_offer_policy_contract(
                &offer.sdp,
                negotiation_config,
                self.session_target_type.as_ref(),
            );
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

            for candidate in remote_candidates {
                let Some(normalized_candidate) =
                    normalize_remote_ice_candidate(&candidate.candidate)
                else {
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
        let Some(data_channel_state) = self.data_channel_state.as_ref().cloned() else {
            return Ok(());
        };

        runtime.block_on(async {
            request_video_keyframe_on_control_channel(&data_channel_state, &control_channel).await
        })
    }

    pub(crate) fn request_decoder_reset(
        &self,
        runtime: &Handle,
    ) -> Result<(), XbxEngineRuntimeError> {
        let Some(control_channel) = self.data_channels.get("control").cloned() else {
            return Ok(());
        };
        let Some(data_channel_state) = self.data_channel_state.as_ref().cloned() else {
            return Ok(());
        };
        runtime.block_on(async {
            request_decoder_reset_on_control_channel(&data_channel_state, &control_channel).await
        })
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
        self.data_channel_state = None;
        self.session_target_type = None;
        if let Some(runtime_stats) = self.runtime_stats.as_ref() {
            if let Ok(mut stats) = runtime_stats.lock() {
                // stop 后立即清掉会污染后续 trace 的瞬态观测，避免 Closed 会话继续冒泡旧数据。
                stats.transport_state = XbxEngineTransportStateDto::Closed;
                stats.session_target_type = None;
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

#[cfg(test)]
mod tests {
    use crate::transport::webrtc::bwe_policy::{resolve_target_remb_kbps, BweDecision};
    use crate::transport::webrtc::recovery::startup_recovery::SessionPhase;
    use crate::transport::webrtc::transport::observation::select_any_candidate_pair_rtt;
    use crate::transport::webrtc::transport::sdp_policy::{
        apply_offer_policy_contract, resolve_offer_video_constraint_tier,
    };
    use crate::{XbxEngineNegotiationRuntimeConfig, XbxEngineWebRtcRuntimeConfig};
    use std::collections::HashMap;
    use tokio::time::Instant;
    use webrtc::stats::{ICECandidatePairStats, RTCStatsType, StatsReport, StatsReportType};
    use xbxengine_protocol::XbxEngineTargetTypeDto;

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
            "m=video 9 UDP/TLS/RTP/SAVPF 102 104 106 108",
            "c=IN IP4 0.0.0.0",
            "a=rtpmap:102 H264/90000",
            "a=fmtp:102 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42001f",
            "a=rtcp-fb:102 transport-cc",
            "a=rtpmap:104 H264/90000",
            "a=fmtp:104 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f",
            "a=rtpmap:106 H264/90000",
            "a=fmtp:106 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=4d0032",
            "a=rtpmap:108 H264/90000",
            "a=fmtp:108 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=640032",
            "a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01",
            "m=application 9 UDP/DTLS/SCTP webrtc-datachannel",
        ]
        .join("\r\n")
    }

    #[test]
    fn resolve_video_constraint_tier_matches_browser_720p_profile() {
        let tier = resolve_offer_video_constraint_tier(
            &XbxEngineNegotiationRuntimeConfig {
                target_resolution_width: 1280,
                target_resolution_height: 720,
                video_bitrate_kbps: 18_000,
                ..Default::default()
            },
            Some(&XbxEngineTargetTypeDto::Home),
        );

        assert_eq!(tier.min_bitrate_kbps, 3_000);
        assert_eq!(tier.start_bitrate_kbps, 10_000);
        assert_eq!(tier.max_bitrate_kbps, 18_000);
        assert_eq!(tier.max_frame_size, 3_600);
    }

    #[test]
    fn resolve_video_constraint_tier_matches_browser_1440p_profile() {
        let tier = resolve_offer_video_constraint_tier(
            &XbxEngineNegotiationRuntimeConfig {
                target_resolution_width: 2560,
                target_resolution_height: 1440,
                video_bitrate_kbps: 60_000,
                ..Default::default()
            },
            Some(&XbxEngineTargetTypeDto::Home),
        );

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
                offer_profile: "64".to_string(),
            },
            Some(&XbxEngineTargetTypeDto::Cloud),
        );

        assert!(patched.contains("m=audio 9 UDP/TLS/RTP/SAVPF 111\r\nc=IN IP4 0.0.0.0\r\nb=AS:192"));
        assert!(patched.contains("useinbandfec=1; stereo=1"));
        assert!(patched.contains("m=video 9 UDP/TLS/RTP/SAVPF 108 106 104 102"));
        assert!(patched.contains("x-google-min-bitrate=8000"));
        assert!(patched.contains("x-google-start-bitrate=25000"));
        assert!(patched.contains("x-google-max-bitrate=40000"));
        assert!(patched.contains("max-fs=14400"));
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
            None,
            SessionPhase::Steady,
            None,
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
            None,
            SessionPhase::Steady,
            None,
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
            None,
            SessionPhase::Steady,
            None,
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
            Some(&XbxEngineTargetTypeDto::Home),
            Some("Direct (host->host)"),
            SessionPhase::Steady,
            None,
            Some(&twcc),
            &mut last_sent_remb_kbps,
            &mut cooldown_ticks,
        );

        assert!(decision.reason.ends_with("ramp-up"));
        assert_eq!(decision.target_kbps, 32_000);
    }

    #[test]
    fn twcc_gcc_direct_path_caps_into_operating_range_under_congestion() {
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
            Some(&XbxEngineTargetTypeDto::Home),
            Some("Direct (host->host)"),
            SessionPhase::Steady,
            None,
            Some(&twcc),
            &mut last_sent_remb_kbps,
            &mut cooldown_ticks,
        );

        assert!(decision.reason.ends_with("congestion-cap"));
        assert_eq!(decision.target_kbps, 32_000);
    }

    #[test]
    fn twcc_gcc_direct_path_allows_burst_but_caps_peak_range() {
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
            observation_id: 3,
            feedback_packet_count: 1,
            covered_sequence_start: 2_000,
            covered_sequence_end: 2_269,
            covered_sequence_span: 270,
            observed_packet_count: 270,
            observed_byte_count: 360_000,
            feedback_interval_ms: Some(100.0),
            arrival_span_ms: Some(94.0),
            receive_bitrate_kbps: Some(29_500.0),
            delivery_ratio: 1.0,
            packet_loss_ratio: 0.0,
            observed_at_ms: 8_000.0,
        };
        let mut last_sent_remb_kbps = 30_000;
        let mut cooldown_ticks = 0;

        let decision = resolve_target_remb_kbps(
            &config,
            None,
            28_000.0,
            0.0,
            Some(&XbxEngineTargetTypeDto::Home),
            Some("Direct (host->host)"),
            SessionPhase::Steady,
            None,
            Some(&twcc),
            &mut last_sent_remb_kbps,
            &mut cooldown_ticks,
        );

        assert!(decision.reason.ends_with("ramp-up"));
        assert_eq!(decision.target_kbps, 37_760);
    }

    #[test]
    fn select_any_candidate_pair_rtt_falls_back_without_selected_pair() {
        let mut reports = HashMap::new();
        reports.insert(
            "pair-a".to_string(),
            StatsReportType::CandidatePair(ICECandidatePairStats {
                timestamp: Instant::now(),
                stats_type: RTCStatsType::CandidatePair,
                id: "pair-a".to_string(),
                local_candidate_id: "local-a".to_string(),
                remote_candidate_id: "remote-a".to_string(),
                state: Default::default(),
                nominated: false,
                packets_sent: 0,
                packets_received: 0,
                bytes_sent: 0,
                bytes_received: 0,
                last_packet_sent_timestamp: Instant::now(),
                last_packet_received_timestamp: Instant::now(),
                total_round_trip_time: 0.0,
                current_round_trip_time: 0.023,
                available_outgoing_bitrate: 0.0,
                available_incoming_bitrate: 0.0,
                requests_received: 0,
                requests_sent: 0,
                responses_received: 0,
                responses_sent: 0,
                consent_requests_sent: 0,
                circuit_breaker_trigger_count: 0,
                consent_expired_timestamp: Instant::now(),
                first_request_timestamp: Instant::now(),
                last_request_timestamp: Instant::now(),
                retransmissions_sent: 0,
            }),
        );
        let stats = StatsReport { reports };

        let rtt = select_any_candidate_pair_rtt(&stats);

        assert_eq!(rtt, Some((0.023, "candidate-pair-any")));
    }
}
