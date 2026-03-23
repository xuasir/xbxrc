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

// 负责 legacy frame pipeline 的挂载和音频播放会话管理，
// 让 stack.rs 只保留生命周期入口。
pub(crate) struct RtcStackMediaPipelineBridge<'a> {
    media_runtime: &'a Arc<tokio::runtime::Runtime>,
    runtime_stats: &'a Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    audio_volume_bits: &'a Arc<AtomicU32>,
    audio_playback_session: &'a Arc<Mutex<Option<XbxRemoteAudioPlaybackSession>>>,
    media: &'a Arc<Mutex<RtcMediaService>>,
    frame_source_tx: &'a Arc<Mutex<Option<FrameSourceSender>>>,
}

impl<'a> RtcStackMediaPipelineBridge<'a> {
    pub(crate) fn new(
        media_runtime: &'a Arc<tokio::runtime::Runtime>,
        runtime_stats: &'a Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        audio_volume_bits: &'a Arc<AtomicU32>,
        audio_playback_session: &'a Arc<Mutex<Option<XbxRemoteAudioPlaybackSession>>>,
        media: &'a Arc<Mutex<RtcMediaService>>,
        frame_source_tx: &'a Arc<Mutex<Option<FrameSourceSender>>>,
    ) -> Self {
        Self {
            media_runtime,
            runtime_stats,
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

    pub(crate) fn mount_legacy_frame_pipeline(&self) {
        let (video_sink, frame_sources) = build_rtc_video_frame_source(
            8192,
            Arc::new(DummyRtcpPort),
            self.runtime_stats.clone(),
            300,
            Duration::from_millis(0),
            Duration::from_millis(50),
            Duration::from_millis(500),
            crate::transport::rtc::stream::nack_scheduler::NackSchedulerConfig {
                max_age_ms: 200,
                frame_deadline_ms: 120,
                burst_count: 4,
                retry_interval_ms: 40,
                max_retry_count: 3,
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
                    "[xbxengine][rtc] legacy frame pipeline mounted and handed to supervisor"
                );
                RuntimeStatsSink::new(self.runtime_stats.clone()).update(|stats| {
                    stats.latest_observation_label =
                        Some("rtcLegacyFramePipelineMounted".to_string());
                    stats.latest_observation_summary =
                        Some("phase1 rtc mounted legacy sample-builder frame pipeline".to_string());
                });
            }
            Some(Err(err)) => {
                crate::xbx_log_info!(
                    "[xbxengine][rtc] legacy frame pipeline mount failed err={err}"
                );
                RuntimeStatsSink::new(self.runtime_stats.clone()).update(|stats| {
                    stats.latest_observation_label =
                        Some("rtcLegacyFramePipelineMountFailed".to_string());
                    stats.latest_observation_summary = Some(format!(
                        "phase1 rtc mount legacy frame pipeline failed err={err}"
                    ));
                });
            }
            None => {
                crate::xbx_log_info!("[xbxengine][rtc] legacy frame pipeline sender unavailable");
                RuntimeStatsSink::new(self.runtime_stats.clone()).update(|stats| {
                    stats.latest_observation_label =
                        Some("rtcLegacyFramePipelineSenderMissing".to_string());
                    stats.latest_observation_summary =
                        Some("phase1 rtc frame source sender unavailable".to_string());
                });
            }
        }
    }
}
