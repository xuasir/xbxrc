use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};

use tokio::time::Duration;

use crate::api::backend::XbxEngineMediaRuntimeStats;
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::stream::audio::{
    build_audio_playback_components, RtcAudioPlaybackSink, XbxRemoteAudioPlaybackSession,
};
use crate::transport::rtc::stream::sink::RtcRtcpSendPort;
use crate::transport::rtc::stream::{build_rtc_video_frame_source, RtcMediaService, RtcMediaSink};
use crate::XbxEngineRuntimeConfig;

pub(super) type FrameSourceSender = tokio::sync::mpsc::Sender<
    crate::transport::rtc::stream::adapter_types::VideoFramePipelineSources,
>;

struct RtcCompositeMediaSink {
    primary: Box<dyn RtcMediaSink>,
    secondary: Box<dyn RtcMediaSink>,
}

impl RtcCompositeMediaSink {
    fn new(primary: Box<dyn RtcMediaSink>, secondary: Box<dyn RtcMediaSink>) -> Self {
        Self { primary, secondary }
    }
}

impl RtcMediaSink for RtcCompositeMediaSink {
    fn apply_payload_route_map(
        &mut self,
        payload_route_map: Option<crate::transport::rtc::stream::packet_router::RtcPayloadRouteMap>,
    ) {
        self.primary
            .apply_payload_route_map(payload_route_map.clone());
        self.secondary.apply_payload_route_map(payload_route_map);
    }

    fn on_raw_packet(
        &mut self,
        packet: &crate::transport::rtc::stream::packet_types::RtcMediaIngressPacket,
        route_label: crate::transport::rtc::stream::packet_router::RtcMediaRouteLabel,
        route_reason: &str,
        rtp_meta: Option<&crate::transport::rtc::stream::packet_types::RtcRtpPacketMeta>,
    ) {
        self.primary
            .on_raw_packet(packet, route_label, route_reason, rtp_meta);
        self.secondary
            .on_raw_packet(packet, route_label, route_reason, rtp_meta);
    }
}

struct DummyRtcpPort;

impl RtcRtcpSendPort for DummyRtcpPort {
    fn send_rtcp(&self, _buf: &[u8]) {}
}

// 负责当前主 frame pipeline 的挂载和音频播放会话管理，
// 让 stack.rs 只保留生命周期入口。
pub(crate) struct RtcStackMediaPipelineBridge<'a> {
    media_runtime: &'a Arc<tokio::runtime::Runtime>,
    runtime_stats: &'a Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    runtime_config: &'a Arc<Mutex<XbxEngineRuntimeConfig>>,
    audio_volume_bits: &'a Arc<AtomicU32>,
    audio_playback_session: &'a Arc<Mutex<Option<XbxRemoteAudioPlaybackSession>>>,
    media: &'a Arc<Mutex<RtcMediaService>>,
    frame_source_tx: &'a Arc<Mutex<Option<FrameSourceSender>>>,
}

impl<'a> RtcStackMediaPipelineBridge<'a> {
    pub(crate) fn new(
        media_runtime: &'a Arc<tokio::runtime::Runtime>,
        runtime_stats: &'a Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        runtime_config: &'a Arc<Mutex<XbxEngineRuntimeConfig>>,
        audio_volume_bits: &'a Arc<AtomicU32>,
        audio_playback_session: &'a Arc<Mutex<Option<XbxRemoteAudioPlaybackSession>>>,
        media: &'a Arc<Mutex<RtcMediaService>>,
        frame_source_tx: &'a Arc<Mutex<Option<FrameSourceSender>>>,
    ) -> Self {
        Self {
            media_runtime,
            runtime_stats,
            runtime_config,
            audio_volume_bits,
            audio_playback_session,
            media,
            frame_source_tx,
        }
    }

    pub(crate) fn stop_audio_playback_session(&self) {
        if let Ok(mut session) = self.audio_playback_session.lock() {
            if let Some(session) = session.take() {
                session.stop();
            }
        }
    }

    pub(crate) fn mount_primary_frame_pipeline(&self) {
        let webrtc = self
            .runtime_config
            .lock()
            .ok()
            .map(|config| config.webrtc.clone())
            .unwrap_or_else(|| XbxEngineRuntimeConfig::default().webrtc);
        let video_pipeline = webrtc.video_pipeline;
        let (video_sink, frame_sources) = build_rtc_video_frame_source(
            8192,
            Arc::new(DummyRtcpPort),
            self.runtime_stats.clone(),
            video_pipeline.jitter_buffer_max_packets.max(64),
            Duration::from_millis(video_pipeline.jitter_buffer_min_delay_ms),
            Duration::from_millis(video_pipeline.jitter_buffer_max_delay_ms),
            Duration::from_millis(video_pipeline.idle_timeout_ms.max(120)),
            crate::transport::rtc::stream::nack_scheduler::NackSchedulerConfig {
                max_age_ms: video_pipeline.nack_max_age_ms,
                frame_deadline_ms: video_pipeline.late_frame_drop_threshold_ms.max(40),
                burst_count: video_pipeline.nack_burst_count.max(1),
                retry_interval_ms: video_pipeline.nack_retry_interval_ms.max(10),
                max_retry_count: video_pipeline.nack_max_retry_count.max(1),
            },
        );
        let mut audio_session = None;
        let audio_sink = match build_audio_playback_components(
            self.media_runtime.handle(),
            self.runtime_stats.clone(),
            self.audio_volume_bits.clone(),
        ) {
            Ok((session, sink)) => {
                audio_session = Some(session);
                sink
            }
            Err(error) => {
                crate::xbx_log_warn!(
                    "[xbxengine][rtc][audio] playback session failed to start: {error}"
                );
                RtcAudioPlaybackSink::disabled()
            }
        };
        if let Ok(mut media) = self.media.lock() {
            media.set_sink(Box::new(RtcCompositeMediaSink::new(
                video_sink,
                Box::new(audio_sink),
            )));
            if let Some(session) = audio_session.take() {
                if let Ok(mut audio_session_guard) = self.audio_playback_session.lock() {
                    *audio_session_guard = Some(session);
                } else {
                    session.stop();
                }
            }
        } else if let Some(session) = audio_session.take() {
            session.stop();
        }
        let send_result = self
            .frame_source_tx
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().cloned())
            .map(|sender| sender.blocking_send(frame_sources));
        match send_result {
            Some(Ok(())) => {
                crate::xbx_log_info!(
                    "[xbxengine][rtc] primary frame pipeline mounted and handed to supervisor"
                );
                RuntimeStatsSink::new(self.runtime_stats.clone()).update(|stats| {
                    stats.latest_observation_label =
                        Some("rtcPrimaryFramePipelineMounted".to_string());
                    stats.latest_observation_summary = Some(
                        "phase1 rtc mounted primary sample-builder frame pipeline".to_string(),
                    );
                });
            }
            Some(Err(err)) => {
                crate::xbx_log_info!(
                    "[xbxengine][rtc] primary frame pipeline mount failed err={err}"
                );
                RuntimeStatsSink::new(self.runtime_stats.clone()).update(|stats| {
                    stats.latest_observation_label =
                        Some("rtcPrimaryFramePipelineMountFailed".to_string());
                    stats.latest_observation_summary = Some(format!(
                        "phase1 rtc mount primary frame pipeline failed err={err}"
                    ));
                });
            }
            None => {
                crate::xbx_log_info!("[xbxengine][rtc] primary frame pipeline sender unavailable");
                RuntimeStatsSink::new(self.runtime_stats.clone()).update(|stats| {
                    stats.latest_observation_label =
                        Some("rtcPrimaryFramePipelineSenderMissing".to_string());
                    stats.latest_observation_summary =
                        Some("phase1 rtc frame source sender unavailable".to_string());
                });
            }
        }
    }
}
