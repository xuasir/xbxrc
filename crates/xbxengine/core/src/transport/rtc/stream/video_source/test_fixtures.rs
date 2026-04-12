use super::sink::RtcVideoSourceSink;
use super::{NackSchedulerConfig, RtcVideoFrameSource};
use crate::api::runtime::XbxEngineRuntimeConfig;
use crate::media::video::test_fixtures::NoopRtcpPort;
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::facts::{
    ConnectionLifecycleStateFact, SessionCommand, TransportCommand,
};
use crate::transport::rtc::projection::{
    BweProjection, ConnectionProjection, DiagnosticsProjection, MediaProjection,
    RecoveryProjection, TransportSnapshot,
};
use crate::transport::rtc::session::actor::SessionPolicyHook;
use crate::transport::rtc::session::policy::RtcSessionPolicy;
use crate::transport::rtc::stream::adapter_types::{FrameSource, TransportObservation};
use crate::transport::rtc::stream::packet_router::parse_payload_route_map_from_answer;
use crate::transport::rtc::stream::packet_router::RtcMediaRouteLabel;
use crate::transport::rtc::stream::packet_types::{
    MediaPacketKind, RtcMediaIngressPacket, RtcMediaPacketSource, RtcRtpPacketMeta,
    RtcVideoRtpPacket,
};
use crate::transport::rtc::stream::sink::{RtcMediaSink, RtcRtcpSendPort};
use crate::{XbxEngineMediaRuntimeStats, XbxEngineVideoTrackStatus};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use xbxengine_protocol::{XbxEngineTargetTypeDto, XbxEngineTransportStateDto};

pub(crate) fn runtime_stats_pair() -> (Arc<Mutex<XbxEngineMediaRuntimeStats>>, RuntimeStatsSink) {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());
    (runtime_stats, sink)
}

pub(crate) fn repair_video_route_map(
) -> Option<crate::transport::rtc::stream::packet_router::RtcPayloadRouteMap> {
    parse_payload_route_map_from_answer(concat!(
        "v=0\r\n",
        "m=video 9 UDP/TLS/RTP/SAVPF 124 97\r\n",
        "a=rtpmap:124 H264/90000\r\n",
        "a=rtpmap:97 rtx/90000\r\n",
        "a=fmtp:97 apt=124\r\n",
        "a=ssrc-group:FID 1111 99\r\n",
    ))
}

pub(crate) fn build_source_with_runtime_stats(
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    rx: tokio::sync::mpsc::Receiver<RtcVideoRtpPacket>,
    transport_observation_tx: tokio::sync::mpsc::UnboundedSender<TransportObservation>,
) -> RtcVideoFrameSource {
    let rtcp_port: Arc<dyn RtcRtcpSendPort> = Arc::new(NoopRtcpPort);
    RtcVideoFrameSource::new(
        rx,
        transport_observation_tx,
        rtcp_port,
        runtime_stats,
        16,
        Duration::from_millis(10),
        Duration::from_millis(20),
        Duration::from_millis(200),
        NackSchedulerConfig {
            max_age_ms: 1_000,
            frame_deadline_ms: 120,
            burst_count: 2,
            retry_interval_ms: 20,
            max_retry_count: 3,
        },
    )
}

pub(crate) struct LocalIngressReplayFixture {
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    sink: Option<RtcVideoSourceSink>,
    source: Option<RtcVideoFrameSource>,
    transport_observation_rx: tokio::sync::mpsc::UnboundedReceiver<TransportObservation>,
}

#[derive(Clone, Debug)]
pub(crate) struct LocalIngressReplayPacket {
    pub payload_type: u8,
    pub sequence_number: u16,
    pub timestamp: u32,
    pub payload: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct LocalIngressHealthyBaseline {
    pub now_ms: f64,
    pub frame_rtp_timestamp: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct LocalIngressReplayProfile {
    pub channel_capacity: usize,
    pub packets: Vec<LocalIngressReplayPacket>,
    pub baseline: LocalIngressHealthyBaseline,
}

impl LocalIngressReplayFixture {
    pub(crate) fn new(channel_capacity: usize) -> Self {
        let (runtime_stats, sink_stats) = runtime_stats_pair();
        let (tx, rx) = tokio::sync::mpsc::channel(channel_capacity);
        let (transport_observation_tx, transport_observation_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let source =
            build_source_with_runtime_stats(runtime_stats.clone(), rx, transport_observation_tx);
        let mut sink = RtcVideoSourceSink::new(tx, sink_stats);
        sink.payload_route_map = repair_video_route_map();
        Self {
            runtime_stats,
            sink: Some(sink),
            source: Some(source),
            transport_observation_rx,
        }
    }

    pub(crate) fn repair_backlog_limit(&self) -> usize {
        self.sink
            .as_ref()
            .expect("sink should still exist")
            .test_repair_backlog_limit()
    }

    pub(crate) fn send_repair_packet(
        &mut self,
        payload_type: u8,
        sequence_number: u16,
        timestamp: u32,
        payload: Vec<u8>,
    ) {
        let packet = RtcMediaIngressPacket::new(
            MediaPacketKind::Rtp,
            payload.len(),
            RtcMediaPacketSource::Track {
                track_id: "video-repair".to_string(),
            },
        )
        .with_rtp_payload(payload);
        let meta = RtcRtpPacketMeta {
            ssrc: 99,
            payload_type,
            sequence_number,
            timestamp,
            marker: false,
        };
        self.sink
            .as_mut()
            .expect("sink should still exist")
            .on_raw_packet(
                &packet,
                RtcMediaRouteLabel::RepairVideo,
                "route=repairVideo",
                Some(&meta),
            );
    }

    pub(crate) fn replay_profile(&mut self, profile: &LocalIngressReplayProfile) {
        for packet in &profile.packets {
            self.send_repair_packet(
                packet.payload_type,
                packet.sequence_number,
                packet.timestamp,
                packet.payload.clone(),
            );
        }
    }

    pub(crate) async fn drain_source_to_completion(&mut self) {
        let mut source = self.source.take().expect("source should still exist");
        let source_task = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_millis(250), source.recv_frame())
                .await
                .expect("source should finish after sink drops")
        });

        let flush_rounds = self.repair_backlog_limit() + 3;
        let mut sink = self.sink.take().expect("sink should still exist");
        for _ in 0..flush_rounds {
            sink.test_flush_pending();
            tokio::task::yield_now().await;
        }
        drop(sink);

        let frame = source_task.await.expect("source task should complete");
        assert!(frame.is_none());
    }

    pub(crate) async fn assert_no_transport_observation(&mut self) {
        assert!(!matches!(
            tokio::time::timeout(
                Duration::from_millis(50),
                self.transport_observation_rx.recv()
            )
            .await,
            Ok(Some(_))
        ));
    }

    pub(crate) fn runtime_stats(&self) -> Arc<Mutex<XbxEngineMediaRuntimeStats>> {
        self.runtime_stats.clone()
    }

    pub(crate) fn seed_healthy_policy_baseline(&self, now_ms: f64, frame_rtp_timestamp: u32) {
        self.seed_healthy_policy_baseline_for_target(
            XbxEngineTargetTypeDto::Cloud,
            now_ms,
            frame_rtp_timestamp,
        );
    }

    pub(crate) fn seed_healthy_policy_baseline_for_target(
        &self,
        target_type: XbxEngineTargetTypeDto,
        now_ms: f64,
        frame_rtp_timestamp: u32,
    ) {
        let mut stats = self.runtime_stats.lock().expect("runtime stats lock");
        stats.session_target_type = Some(target_type);
        stats.transport_state = XbxEngineTransportStateDto::Connected;
        stats.latest_video_host_present_time_ms = Some(now_ms - 16.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 12.0);
        stats.latest_video_packet_arrival_time_ms = Some(now_ms - 10.0);
        stats.host_no_pending_pressure_level = Some("normal".to_string());
        stats.host_no_pending_streak = 0;
        stats.video_present_submit_count_total = 120;
        stats.video_present_drop_count_total = 0;
        stats.video_present_overwrite_count_total = 0;
        stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1280),
            video_height: Some(720),
            mime_type: Some("video/H264".to_string()),
            transport_state: XbxEngineTransportStateDto::Connected,
            video_bytes_total: 32_000,
            video_packet_count_total: 240,
            audio_bytes_total: 1_024,
            observed_at_ms: now_ms,
        });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-complete-candidate".to_string(),
            gap: None,
            frame: Some(crate::XbxEngineVideoTimelineFrameSnapshot {
                state: "complete-candidate".to_string(),
                frame_rtp_timestamp: Some(frame_rtp_timestamp),
                is_keyframe: Some(false),
                frame_importance: Some("delta".to_string()),
                budget_importance: None,

                evidence_importance: None,

                close_reason: None,
                observed_at_ms: now_ms,
            }),
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "healthy".to_string(),
                reason: None,
                chain_break_evidence: None,

                observed_at_ms: now_ms,
            },
            observed_at_ms: now_ms,
        });
    }

    pub(crate) fn run_policy_snapshot(&self, now_ms: f64) -> Vec<TransportCommand> {
        let mut policy = RtcSessionPolicy::new(
            Arc::new(Mutex::new(XbxEngineRuntimeConfig::default())),
            self.runtime_stats(),
        );
        let snapshot = self.build_connected_snapshot(1, now_ms, 240, "none");
        policy
            .on_snapshot(&snapshot)
            .into_iter()
            .filter_map(|command| match command {
                SessionCommand::Transport(command) => Some(command),
                SessionCommand::LocalDecoderReset { .. } => None,
            })
            .collect()
    }

    pub(crate) fn build_connected_snapshot(
        &self,
        observation_id: u64,
        now_ms: f64,
        frame_count: u64,
        diagnosis_label: &str,
    ) -> TransportSnapshot {
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        connection.control_channel_open = true;
        connection.latest_transport_path = Some("Direct".to_string());
        connection.latest_rtt_ms = Some(42.0);
        connection.last_observed_at_ms = Some(now_ms);
        TransportSnapshot::new(
            observation_id,
            now_ms,
            connection,
            MediaProjection {
                frame_count,
                ..MediaProjection::default()
            },
            RecoveryProjection {
                latest_diagnosis_label: Some(diagnosis_label.to_string()),
                pending_action: false,
                successful_action_count: 0,
                failed_action_count: 0,
                last_observed_at_ms: Some(now_ms),
                ..Default::default()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        )
    }

    pub(crate) fn mark_transport_connectivity_degraded(&self, observed_at_ms: f64) {
        let mut stats = self.runtime_stats.lock().expect("runtime stats lock");
        stats.transport_state = XbxEngineTransportStateDto::Disconnected;
        stats.host_no_pending_pressure_level = Some("critical".to_string());
        stats.host_no_pending_streak = 380;
        stats.latest_video_host_present_time_ms = Some(observed_at_ms - 1_400.0);
        stats.latest_video_decode_ok_time_ms = Some(observed_at_ms - 900.0);
        stats.latest_video_packet_arrival_time_ms = Some(observed_at_ms - 640.0);
        stats.video_renderer_stalled = Some(true);
        stats.video_decoder_stalled = Some(false);
        stats.video_anchor_clean_epoch = None;
        stats.video_anchor_clean_observed_at_ms = None;
        stats.video_anchor_clean_source_event = None;
        if let Some(track) = stats.latest_video_track_status.as_mut() {
            track.transport_state = XbxEngineTransportStateDto::Disconnected;
            track.observed_at_ms = observed_at_ms;
        }
        if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
            timeline.source_event = "frame-await-recovery-keyframe".to_string();
            timeline.chain.state = "recovering".to_string();
            timeline.chain.reason = Some("transportAwaitRecoveryKeyframe".to_string());
            timeline.chain.observed_at_ms = observed_at_ms;
            timeline.observed_at_ms = observed_at_ms;
        }
    }

    pub(crate) fn mark_transport_recovered(&self, observed_at_ms: f64) {
        let mut stats = self.runtime_stats.lock().expect("runtime stats lock");
        stats.transport_recovery_epoch = stats.transport_recovery_epoch.saturating_add(1);
        stats.transport_state = XbxEngineTransportStateDto::Connected;
        stats.host_no_pending_pressure_level = Some("normal".to_string());
        stats.host_no_pending_streak = 0;
        stats.latest_video_host_present_time_ms = Some(observed_at_ms - 10.0);
        stats.latest_video_decode_ok_time_ms = Some(observed_at_ms - 6.0);
        stats.latest_video_packet_arrival_time_ms = Some(observed_at_ms - 4.0);
        stats.video_renderer_stalled = Some(false);
        stats.video_decoder_stalled = Some(false);
        stats.video_anchor_clean_epoch = Some(stats.transport_recovery_epoch);
        stats.video_anchor_clean_observed_at_ms = Some(observed_at_ms);
        stats.video_anchor_clean_source_event = Some("chain-clean-keyframe-submitted".to_string());
        if let Some(track) = stats.latest_video_track_status.as_mut() {
            track.transport_state = XbxEngineTransportStateDto::Connected;
            track.video_bytes_total += 32_000;
            track.video_packet_count_total += 240;
            track.observed_at_ms = observed_at_ms;
        }
        if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
            timeline.source_event = "frame-observed".to_string();
            timeline.chain.state = "healthy".to_string();
            timeline.chain.reason = None;
            timeline.chain.observed_at_ms = observed_at_ms;
            timeline.observed_at_ms = observed_at_ms;
        }
    }
}

pub(crate) async fn run_local_ingress_replay_profile(
    profile: &LocalIngressReplayProfile,
) -> LocalIngressReplayFixture {
    let mut fixture = LocalIngressReplayFixture::new(profile.channel_capacity);
    fixture.replay_profile(profile);
    fixture.drain_source_to_completion().await;
    fixture.assert_no_transport_observation().await;
    fixture.seed_healthy_policy_baseline(
        profile.baseline.now_ms,
        profile.baseline.frame_rtp_timestamp,
    );
    fixture
}
