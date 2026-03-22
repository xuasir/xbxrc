use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;

use crate::{
    media::video::ingress::{
        budget::materialize_ingress_frame,
        scheduler::{FrameScheduler, IngressDecision, VideoIngress},
    },
    media::video::render::renderer::XbxRenderState,
    media::video::types::AssembledVideoFrame,
    runtime_stats_sink::RuntimeStatsSink,
    transport::rtc::facts::{IngressDecisionFact, MediaFact, TransportFact},
    transport::rtc::media::adapter_types::{
        TransportAdmissionObservation, TransportLossObservation, TransportObservation,
        VideoFramePipelineSources,
    },
    XbxEngineMediaRuntimeStats, XbxEngineRuntimeConfig,
};

use super::observation::MediaSupervisorObservationState;

#[derive(Clone)]
pub(super) struct MediaSessionContext {
    pub(super) runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    pub(super) render_state: Arc<Mutex<XbxRenderState>>,
    pub(super) transport_fact_sink: Arc<Mutex<Vec<TransportFact>>>,
    pub(super) runtime_config: XbxEngineRuntimeConfig,
}

pub(super) struct ActiveMediaSession {
    decode: Arc<crate::media::video::decode::actor::DecodeActorHandle>,
    pacer: Arc<crate::media::video::pacer::actor::PacerActorHandle>,
    renderer: Arc<crate::media::video::render::actor::RendererActorHandle>,
    frame_source_task: tokio::task::JoinHandle<()>,
    transport_observation_task: tokio::task::JoinHandle<()>,
    task: tokio::task::JoinHandle<()>,
}

impl ActiveMediaSession {
    pub(super) fn stop(self) {
        self.decode.stop();
        self.pacer.stop();
        self.renderer.stop();
        self.frame_source_task.abort();
        self.transport_observation_task.abort();
        self.task.abort();
    }
}

pub(super) fn spawn_media_session(
    sources: VideoFramePipelineSources,
    context: MediaSessionContext,
) -> ActiveMediaSession {
    crate::xbx_log_info!("[MediaSession] spawning video session loop");
    let renderer_handle = Arc::new(
        crate::media::video::render::actor::RendererActorHandle::new(
            context.render_state.clone(),
            context.runtime_stats.clone(),
        ),
    );
    let pacer_handle = Arc::new(crate::media::video::pacer::actor::PacerActorHandle::new(
        renderer_handle.clone(),
        context.runtime_stats.clone(),
        16,
    ));
    let video_pipeline_config = context.runtime_config.webrtc.video_pipeline.clone();
    let jitter_buffer_min_delay =
        Duration::from_millis(video_pipeline_config.jitter_buffer_min_delay_ms);
    let jitter_buffer_max_delay =
        Duration::from_millis(video_pipeline_config.jitter_buffer_max_delay_ms);
    let decode_handle = Arc::new(crate::media::video::decode::actor::DecodeActorHandle::new(
        pacer_handle.clone(),
        context.runtime_stats.clone(),
        video_pipeline_config.jitter_buffer_min_delay_ms,
        video_pipeline_config.jitter_buffer_max_delay_ms,
    ));
    let severe_deadline_packet_threshold =
        (usize::from(video_pipeline_config.nack_burst_count.max(1)) * 32).max(128);

    let (frame_tx, frame_rx) = mpsc::channel::<AssembledVideoFrame>(256);
    let (transport_observation_tx, transport_observation_rx) =
        mpsc::unbounded_channel::<TransportObservation>();
    let VideoFramePipelineSources {
        mut frame_source,
        mut transport_observation_source,
    } = sources;

    let frame_source_task = tokio::spawn(async move {
        crate::xbx_log_info!("[MediaSession] frame feeder started");
        while let Some(frame) = frame_source.recv_frame().await {
            if frame_tx.send(frame).await.is_err() {
                break;
            }
        }
        crate::xbx_log_info!("[MediaSession] frame feeder stopped");
    });

    let transport_observation_task = tokio::spawn(async move {
        crate::xbx_log_info!("[MediaSession] transport observation feeder started");
        while let Some(observation) = transport_observation_source
            .recv_transport_observation()
            .await
        {
            if transport_observation_tx.send(observation).is_err() {
                break;
            }
        }
        crate::xbx_log_info!("[MediaSession] transport observation feeder stopped");
    });

    let session_decode_handle = decode_handle.clone();

    let task = {
        tokio::spawn(async move {
            MediaSessionLoop::new(
                frame_rx,
                transport_observation_rx,
                jitter_buffer_min_delay,
                jitter_buffer_max_delay,
                video_pipeline_config.late_frame_drop_threshold_ms,
                video_pipeline_config.backlog_drop_threshold_packets,
                session_decode_handle,
                context.runtime_stats.clone(),
                context.transport_fact_sink.clone(),
                severe_deadline_packet_threshold,
            )
            .run()
            .await;
        })
    };

    ActiveMediaSession {
        decode: decode_handle,
        pacer: pacer_handle,
        renderer: renderer_handle,
        frame_source_task,
        transport_observation_task,
        task,
    }
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
    decode_drain_tick: tokio::time::Interval,
    frame_event_count: u64,
    transport_event_count: u64,
    decode_tick_count: u64,
}

impl MediaSessionLoop {
    fn new(
        frame_rx: mpsc::Receiver<AssembledVideoFrame>,
        transport_observation_rx: mpsc::UnboundedReceiver<TransportObservation>,
        jitter_buffer_min_delay: Duration,
        jitter_buffer_max_delay: Duration,
        late_frame_drop_threshold_ms: u64,
        backlog_drop_threshold_packets: u16,
        decode_handle: Arc<crate::media::video::decode::actor::DecodeActorHandle>,
        runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        transport_fact_sink: Arc<Mutex<Vec<TransportFact>>>,
        severe_deadline_packet_threshold: usize,
    ) -> Self {
        Self {
            frame_rx,
            transport_observation_rx,
            ingress: VideoIngress::new(
                usize::from(backlog_drop_threshold_packets.max(1)),
                Duration::from_millis(late_frame_drop_threshold_ms),
            ),
            decode_handle,
            runtime_stats: RuntimeStatsSink::new(runtime_stats),
            transport_fact_sink,
            observation: MediaSupervisorObservationState::new(),
            jitter_buffer_min_delay,
            jitter_buffer_max_delay,
            severe_deadline_packet_threshold,
            decode_drain_tick: tokio::time::interval(Duration::from_millis(4)),
            frame_event_count: 0,
            transport_event_count: 0,
            decode_tick_count: 0,
        }
    }

    async fn run(mut self) {
        crate::xbx_log_info!("[MediaSession] loop started");
        loop {
            tokio::select! {
                _ = self.decode_drain_tick.tick() => {
                    self.decode_tick_count = self.decode_tick_count.saturating_add(1);
                    if self.decode_tick_count == 1 || self.decode_tick_count.is_power_of_two() {
                        crate::xbx_log_info!(
                            "[MediaSession] decode drain tick count={}",
                            self.decode_tick_count
                        );
                    }
                    self.on_decode_drain_tick();
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

    fn on_decode_drain_tick(&mut self) {
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
        let reconfigure_reason = self.ingress.describe_reconfigure_reason(&encoded_frame);
        let decision = self.ingress.submit(encoded_frame, Instant::now());
        self.observation.record_ingress_observation(
            &self.runtime_stats,
            &decision,
            reconfigure_reason.as_deref(),
            now_ms,
            frame_meta.0,
            frame_meta.1,
            frame_meta.2,
            frame_queue_depth_before_submit,
        );
        self.push_transport_fact(TransportFact::Media(MediaFact::IngressDecisionObserved {
            decision: map_ingress_decision_fact(&decision),
            queue_depth: frame_queue_depth_before_submit,
            observed_at_ms: now_ms,
        }));
        if matches!(
            decision,
            IngressDecision::WaitKeyframe | IngressDecision::Reconfigure
        ) {
            if self.frame_event_count == 1 || self.frame_event_count.is_power_of_two() {
                crate::xbx_log_warn!(
                    "[MediaSession] frame event triggered ingress hint={:?}",
                    decision
                );
            }
        }
        self.on_decode_drain_tick();
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
    }

    fn push_transport_fact(&self, fact: TransportFact) {
        if let Ok(mut pending) = self.transport_fact_sink.lock() {
            pending.push(fact);
        }
    }
}

fn map_transport_observation_to_hint_label(
    observation: &TransportObservation,
    severe_deadline_packet_threshold: usize,
) -> &'static str {
    match observation {
        TransportObservation::Admission(TransportAdmissionObservation::AwaitRecoveryKeyframe) => {
            "transportAwaitRecoveryKeyframe"
        }
        TransportObservation::Loss(TransportLossObservation::PacketLossDetected) => {
            "transportSampleLoss"
        }
        TransportObservation::Loss(TransportLossObservation::RecoveryKeyframeRequested) => {
            "transportSampleLossBurst"
        }
        TransportObservation::Loss(TransportLossObservation::AwaitRecoveryKeyframe) => {
            "transportAwaitRecoveryKeyframe"
        }
        TransportObservation::StreamIdleTimeout => "adapterIdleTimeout",
        TransportObservation::StreamThinStall => "adapterThinStream",
        TransportObservation::NackRecoveredLate => "transportRecoveredLate",
        TransportObservation::NackDeadlineExpired { missing_packets } => {
            if usize::from(*missing_packets) >= severe_deadline_packet_threshold {
                "transportSevereDeadline"
            } else {
                "transportExpiredDeadline"
            }
        }
    }
}

fn map_ingress_decision_fact(decision: &IngressDecision) -> IngressDecisionFact {
    match decision {
        IngressDecision::Submit => IngressDecisionFact::Submit,
        IngressDecision::DropLate => IngressDecisionFact::DropLate,
        IngressDecision::DropBacklog => IngressDecisionFact::DropBacklog,
        IngressDecision::WaitKeyframe => IngressDecisionFact::WaitKeyframe,
        IngressDecision::Reconfigure => IngressDecisionFact::Reconfigure,
    }
}

fn transport_observation_severity(observation: &TransportObservation) -> u8 {
    match observation {
        TransportObservation::NackDeadlineExpired { missing_packets } if *missing_packets >= 64 => {
            2
        }
        TransportObservation::StreamIdleTimeout
        | TransportObservation::StreamThinStall
        | TransportObservation::NackDeadlineExpired { .. } => 1,
        TransportObservation::Admission(_)
        | TransportObservation::Loss(_)
        | TransportObservation::NackRecoveredLate => 0,
    }
}

fn drain_ingress_to_decode(
    ingress: &mut VideoIngress,
    decode_handle: &Arc<crate::media::video::decode::actor::DecodeActorHandle>,
    runtime_stats: &RuntimeStatsSink,
    frame_count: u64,
    observation: &MediaSupervisorObservationState,
) {
    while decode_handle.available_slots() > 0 {
        let Some(frame) = ingress.pop() else {
            break;
        };
        let frame_w = frame.width;
        let frame_h = frame.height;
        let frame_is_key = frame.is_keyframe;
        let frame_payload_len = frame.payload.len();

        observation.record_stream_dimensions(runtime_stats, frame_w, frame_h);

        if let Err(error) = decode_handle.submit(frame) {
            let frame = match error {
                crate::media::video::decode::actor::DecodeSubmitError::Full(frame)
                | crate::media::video::decode::actor::DecodeSubmitError::Disconnected(frame) => {
                    frame
                }
            };
            ingress.requeue_front(frame);
            crate::xbx_log_warn!(
                "[MediaSession] decode queue unavailable, keep remaining ingress backlog"
            );
            break;
        }

        let frame_type = if frame_is_key { "KEYFRAME" } else { "DELTA" };
        if frame_is_key || frame_count % 300 == 0 {
            let (w, h) = if frame_w > 0 {
                (frame_w, frame_h)
            } else {
                runtime_stats
                    .read(|stats| {
                        stats
                            .latest_video_stream_width
                            .zip(stats.latest_video_stream_height)
                    })
                    .flatten()
                    .unwrap_or((0, 0))
            };
            crate::xbx_log_info!(
                "[MediaSession] received valid {} frame (size: {}B, res: {}x{}, frame#: {})",
                frame_type,
                frame_payload_len,
                w,
                h,
                frame_count
            );
        }
    }
}

fn now_ms_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}
