use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;

use crate::{
    media::video::ingress::{
        budget::materialize_ingress_frame,
        scheduler::{FrameScheduler, IngressDecision, VideoIngress},
    },
    media::video::types::AssembledVideoFrame,
    runtime_stats_sink::RuntimeStatsSink,
    transport::rtc::facts::{IngressDecisionFact, MediaFact, TransportFact},
    transport::rtc::recovery::escalation::VideoEscalationReason,
    transport::rtc::stream::adapter_types::TransportObservation,
    XbxEngineMediaRuntimeStats,
};

use super::ingress::drain_ingress_to_decode;
use super::observation::{
    map_transport_observation_to_hint_label, transport_observation_severity,
    MediaSupervisorObservationState,
};

pub(super) struct MediaSessionLoopConfig {
    pub(super) jitter_buffer_min_delay: Duration,
    pub(super) jitter_buffer_max_delay: Duration,
    pub(super) late_frame_drop_threshold_ms: u64,
    pub(super) backlog_drop_threshold_packets: u16,
    pub(super) severe_deadline_packet_threshold: usize,
}

pub(super) fn spawn_media_session_loop(
    frame_rx: mpsc::Receiver<AssembledVideoFrame>,
    transport_observation_rx: mpsc::UnboundedReceiver<TransportObservation>,
    decode_handle: Arc<crate::media::video::decode::actor::DecodeActorHandle>,
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    transport_fact_sink: Arc<Mutex<Vec<TransportFact>>>,
    config: MediaSessionLoopConfig,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        MediaSessionLoop::new(
            frame_rx,
            transport_observation_rx,
            decode_handle,
            runtime_stats,
            transport_fact_sink,
            config,
        )
        .run()
        .await;
    })
}

struct MediaSessionLoop {
    frame_rx: mpsc::Receiver<AssembledVideoFrame>,
    transport_observation_rx: mpsc::UnboundedReceiver<TransportObservation>,
    ingress: VideoIngress,
    decode_handle: Arc<crate::media::video::decode::actor::DecodeActorHandle>,
    runtime_stats: RuntimeStatsSink,
    transport_fact_sink: Arc<Mutex<Vec<TransportFact>>>,
    observation: MediaSupervisorObservationState,
    jitter_buffer_min_delay: Duration,
    jitter_buffer_max_delay: Duration,
    severe_deadline_packet_threshold: usize,
    decode_demand_epoch: u64,
    frame_event_count: u64,
    transport_event_count: u64,
}

impl MediaSessionLoop {
    fn new(
        frame_rx: mpsc::Receiver<AssembledVideoFrame>,
        transport_observation_rx: mpsc::UnboundedReceiver<TransportObservation>,
        decode_handle: Arc<crate::media::video::decode::actor::DecodeActorHandle>,
        runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        transport_fact_sink: Arc<Mutex<Vec<TransportFact>>>,
        config: MediaSessionLoopConfig,
    ) -> Self {
        let decode_demand_epoch = decode_handle.demand_epoch();
        Self {
            frame_rx,
            transport_observation_rx,
            ingress: VideoIngress::new(
                usize::from(config.backlog_drop_threshold_packets.max(1)),
                Duration::from_millis(config.late_frame_drop_threshold_ms),
            ),
            decode_handle,
            runtime_stats: RuntimeStatsSink::new(runtime_stats),
            transport_fact_sink,
            observation: MediaSupervisorObservationState::new(),
            jitter_buffer_min_delay: config.jitter_buffer_min_delay,
            jitter_buffer_max_delay: config.jitter_buffer_max_delay,
            severe_deadline_packet_threshold: config.severe_deadline_packet_threshold,
            decode_demand_epoch,
            frame_event_count: 0,
            transport_event_count: 0,
        }
    }

    async fn run(mut self) {
        crate::xbx_log_info!("[MediaSession] loop started");
        loop {
            tokio::select! {
                changed_epoch = self
                    .decode_handle
                    .wait_for_demand_change_since(self.decode_demand_epoch) => {
                    self.decode_demand_epoch = changed_epoch;
                    self.drive_decode_pull();
                }
                maybe_frame = self.frame_rx.recv() => {
                    let Some(frame) = maybe_frame else {
                        crate::xbx_log_info!("[MediaSession] frame source connection closed");
                        break;
                    };
                    self.frame_event_count = self.frame_event_count.saturating_add(1);
                    if self.frame_event_count == 1 || self.frame_event_count.is_power_of_two() {
                        crate::xbx_log_info!(
                            "[MediaSession] frame event count={}",
                            self.frame_event_count
                        );
                    }
                    self.on_frame(frame).await;
                }
                maybe_observation = self.transport_observation_rx.recv() => {
                    let Some(observation) = maybe_observation else {
                        crate::xbx_log_info!("[MediaSession] transport observation source closed");
                        break;
                    };
                    self.transport_event_count = self.transport_event_count.saturating_add(1);
                    if self.transport_event_count == 1 || self.transport_event_count.is_power_of_two()
                    {
                        crate::xbx_log_info!(
                            "[MediaSession] transport observation count={}",
                            self.transport_event_count
                        );
                    }
                    self.on_transport_observation(observation).await;
                }
            }
        }
    }

    fn drive_decode_pull(&mut self) {
        drain_ingress_to_decode(
            &mut self.ingress,
            &self.decode_handle,
            &self.runtime_stats,
            self.observation.total_frame_count(),
            &self.observation,
        );
    }

    async fn on_frame(&mut self, assembled_frame: AssembledVideoFrame) {
        use std::time::Instant;

        let now_ms = now_ms_f64();
        self.observation
            .record_frame_arrival(&self.runtime_stats, now_ms);
        self.push_transport_fact(TransportFact::Media(MediaFact::FrameArrived {
            rtp_timestamp: assembled_frame.rtp_timestamp,
            width: assembled_frame.width,
            height: assembled_frame.height,
            is_keyframe: assembled_frame.is_keyframe,
            observed_at_ms: now_ms,
        }));
        self.runtime_stats
            .record_picture_recovery_episode_packet_seen(
                now_ms,
                Some(assembled_frame.rtp_timestamp),
                assembled_frame.is_keyframe,
                assembled_frame.first_packet_sequence,
            );
        if self.frame_event_count == 1 || self.frame_event_count.is_power_of_two() {
            crate::xbx_log_info!(
                "[MediaSession] frame ts={} size={}x{} keyframe={}",
                assembled_frame.rtp_timestamp,
                assembled_frame.width,
                assembled_frame.height,
                assembled_frame.is_keyframe
            );
        }

        let encoded_frame = materialize_ingress_frame(
            assembled_frame,
            self.jitter_buffer_min_delay,
            self.jitter_buffer_max_delay,
        );
        let frame_queue_depth_before_submit = self.ingress.queue_depth();
        let frame_meta = (
            encoded_frame.width,
            encoded_frame.height,
            encoded_frame.is_keyframe,
        );
        let frame_rtp_timestamp = encoded_frame.rtp_timestamp;
        let frame_recovery_disposition = encoded_frame.frame_recovery_disposition;
        let frame_unrecoverable_reason = encoded_frame.frame_unrecoverable_reason.clone();
        let ingress_reason = encoded_frame
            .frame_recovery_disposition
            .ingress_reason()
            .or_else(|| encoded_frame.frame_unrecoverable_reason.as_deref())
            .map(str::to_string);
        let reconfigure_reason = self.ingress.describe_reconfigure_reason(&encoded_frame);
        let decision = self.ingress.submit(encoded_frame, Instant::now());
        self.observation.record_ingress_observation(
            &self.runtime_stats,
            &decision,
            ingress_reason.as_deref(),
            reconfigure_reason.as_deref(),
            now_ms,
            frame_meta.0,
            frame_meta.1,
            frame_meta.2,
            frame_queue_depth_before_submit,
            Some(frame_rtp_timestamp),
            None,
            Some(frame_recovery_disposition),
            frame_unrecoverable_reason.as_deref(),
        );
        self.push_transport_fact(TransportFact::Media(MediaFact::IngressDecisionObserved {
            decision: IngressDecisionFact::from(&decision),
            queue_depth: frame_queue_depth_before_submit,
            observed_at_ms: now_ms,
        }));
        if matches!(
            decision,
            IngressDecision::WaitKeyframe
                | IngressDecision::DropUnrecoverable
                | IngressDecision::Reconfigure
        ) {
            // 直接从decision映射到reason，不再经过signal/diagnosis两层
            let reason = match &decision {
                IngressDecision::WaitKeyframe => VideoEscalationReason::WaitKeyframe,
                IngressDecision::Reconfigure => VideoEscalationReason::Reconfigure,
                IngressDecision::DropUnrecoverable => VideoEscalationReason::LocalSupplySuspect,
                _ => VideoEscalationReason::WaitKeyframe,
            };
            let hint_label = reason.label();

            if self
                .observation
                .should_log_transport_hint(hint_label, now_ms)
            {
                self.observation
                    .record_transport_hint(hint_label.to_string(), now_ms);
            }
            self.push_transport_fact(TransportFact::Media(
                MediaFact::TransportObservationRaised {
                    label: hint_label.to_string(),
                    severity: 1,
                    observed_at_ms: now_ms,
                },
            ));
            if self.frame_event_count == 1 || self.frame_event_count.is_power_of_two() {
                crate::xbx_log_warn!(
                    "[MediaSession] frame event triggered ingress hint={:?}",
                    decision
                );
            }
        }
        self.drive_decode_pull();
    }

    async fn on_transport_observation(&mut self, observation: TransportObservation) {
        let hint_label = map_transport_observation_to_hint_label(
            &observation,
            self.severe_deadline_packet_threshold,
        );
        let hint_now_ms = now_ms_f64();
        if self.transport_event_count == 1 || self.transport_event_count.is_power_of_two() {
            crate::xbx_log_info!(
                "[MediaSession] transport observation diagnosis={}",
                hint_label
            );
        }
        if self
            .observation
            .should_log_transport_hint(hint_label, hint_now_ms)
        {
            crate::xbx_log_warn!("[MediaSession] Transport escalation hint: {}", hint_label);
            self.observation
                .record_transport_hint(hint_label.to_string(), hint_now_ms);
        }
        self.push_transport_fact(TransportFact::Media(
            MediaFact::TransportObservationRaised {
                label: hint_label.to_string(),
                severity: transport_observation_severity(&observation),
                observed_at_ms: hint_now_ms,
            },
        ));
        self.drive_decode_pull();
    }

    // 这里只保留 fact 写入口，避免 session 壳层再感知内部事件细节。
    fn push_transport_fact(&self, fact: TransportFact) {
        if let Ok(mut pending) = self.transport_fact_sink.lock() {
            pending.push(fact);
        }
    }
}

fn now_ms_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}
