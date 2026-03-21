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
    transport::rtc::media::adapter_types::{
        TransportAdmissionObservation, TransportLossObservation, TransportObservation,
        VideoFramePipelineSources,
    },
    transport::rtc::recovery::escalation::VideoEscalationController,
    XbxEngineMediaRuntimeStats, XbxEnginePendingRuntimeRecoveryAction, XbxEngineRuntimeConfig,
};

use super::observation::MediaSupervisorObservationState;
use super::recovery_driver::RecoveryDriver;
use super::recovery_types::RecoverySchedulerInput;
use super::scheduler::MediaSessionScheduler;

#[derive(Clone)]
pub(super) struct MediaSessionContext {
    pub(super) runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    pub(super) pending_runtime_recovery_action:
        Arc<Mutex<Option<XbxEnginePendingRuntimeRecoveryAction>>>,
    pub(super) data_channel_state: Arc<Mutex<crate::transport::rtc::protocol::data_channel_state::XbxDataChannelState>>,
    pub(super) render_state: Arc<Mutex<XbxRenderState>>,
    pub(super) runtime_config: XbxEngineRuntimeConfig,
}

pub(super) struct ActiveMediaSession {
    decode: Arc<crate::media::video::decode::actor::DecodeActorHandle>,
    pacer: Arc<crate::media::video::pacer::actor::PacerActorHandle>,
    renderer: Arc<crate::media::video::render::actor::RendererActorHandle>,
    frame_source_task: tokio::task::JoinHandle<()>,
    transport_observation_task: tokio::task::JoinHandle<()>,
    recovery_task: tokio::task::JoinHandle<()>,
    task: tokio::task::JoinHandle<()>,
}

impl ActiveMediaSession {
    pub(super) fn stop(self) {
        self.decode.stop();
        self.pacer.stop();
        self.renderer.stop();
        self.frame_source_task.abort();
        self.transport_observation_task.abort();
        self.recovery_task.abort();
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
    let (recovery_input_tx, recovery_input_rx) =
        mpsc::unbounded_channel::<RecoverySchedulerInput>();

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
        while let Some(observation) = transport_observation_source.recv_transport_observation().await {
            if transport_observation_tx.send(observation).is_err() {
                break;
            }
        }
        crate::xbx_log_info!("[MediaSession] transport observation feeder stopped");
    });

    let recovery_task = spawn_media_recovery(
        recovery_input_rx,
        context.clone(),
        decode_handle.clone(),
    );
    let session_decode_handle = decode_handle.clone();

    let task = {
        tokio::spawn(async move {
            MediaSessionLoop::new(
                frame_rx,
                transport_observation_rx,
                recovery_input_tx,
                jitter_buffer_min_delay,
                jitter_buffer_max_delay,
                video_pipeline_config.late_frame_drop_threshold_ms,
                video_pipeline_config.backlog_drop_threshold_packets,
                session_decode_handle,
                context.runtime_stats.clone(),
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
        recovery_task,
        task,
    }
}

fn spawn_media_recovery(
    mut recovery_input_rx: mpsc::UnboundedReceiver<RecoverySchedulerInput>,
    context: MediaSessionContext,
    decode_handle: Arc<crate::media::video::decode::actor::DecodeActorHandle>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        crate::xbx_log_info!("[Recovery] dispatcher started");
        let stream_started_at = std::time::Instant::now();
        let startup_escalation_grace =
            Duration::from_millis(context.runtime_config.webrtc.recovery.first_frame_grace_ms);
        let recovery_driver = RecoveryDriver::new(
            VideoEscalationController::new(
                ingress_keyframe_cooldown_from_context(&context),
                context
                    .runtime_config
                    .webrtc
                    .recovery
                    .keyframe_loss_burst_threshold
                    .max(1) as u8,
                context
                    .runtime_config
                    .webrtc
                    .recovery
                    .keyframe_loss_burst_threshold
                    .saturating_add(1)
                    .max(1),
            ),
            context.data_channel_state.clone(),
            context.pending_runtime_recovery_action.clone(),
            context.runtime_stats.clone(),
            decode_handle,
            stream_started_at,
            startup_escalation_grace,
        );
        let mut scheduler = MediaSessionScheduler::new(recovery_driver);
        while let Some(input) = recovery_input_rx.recv().await {
            scheduler.handle_input(input).await;
        }
        crate::xbx_log_info!("[Recovery] dispatcher stopped");
    })
}

struct MediaSessionLoop {
    frame_rx: mpsc::Receiver<AssembledVideoFrame>,
    transport_observation_rx: mpsc::UnboundedReceiver<TransportObservation>,
    recovery_input_tx: mpsc::UnboundedSender<RecoverySchedulerInput>,
    ingress: VideoIngress,
    decode_handle: Arc<crate::media::video::decode::actor::DecodeActorHandle>,
    runtime_stats: RuntimeStatsSink,
    observation: MediaSupervisorObservationState,
    jitter_buffer_min_delay: Duration,
    jitter_buffer_max_delay: Duration,
    severe_deadline_packet_threshold: usize,
    startup_retry_tick: tokio::time::Interval,
    decode_drain_tick: tokio::time::Interval,
    frame_event_count: u64,
    transport_event_count: u64,
    decode_tick_count: u64,
    startup_retry_tick_count: u64,
}

impl MediaSessionLoop {
    fn new(
        frame_rx: mpsc::Receiver<AssembledVideoFrame>,
        transport_observation_rx: mpsc::UnboundedReceiver<TransportObservation>,
        recovery_input_tx: mpsc::UnboundedSender<RecoverySchedulerInput>,
        jitter_buffer_min_delay: Duration,
        jitter_buffer_max_delay: Duration,
        late_frame_drop_threshold_ms: u64,
        backlog_drop_threshold_packets: u16,
        decode_handle: Arc<crate::media::video::decode::actor::DecodeActorHandle>,
        runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        severe_deadline_packet_threshold: usize,
    ) -> Self {
        Self {
            frame_rx,
            transport_observation_rx,
            recovery_input_tx,
            ingress: VideoIngress::new(
                usize::from(backlog_drop_threshold_packets.max(1)),
                Duration::from_millis(late_frame_drop_threshold_ms),
            ),
            decode_handle,
            runtime_stats: RuntimeStatsSink::new(runtime_stats),
            observation: MediaSupervisorObservationState::new(),
            jitter_buffer_min_delay,
            jitter_buffer_max_delay,
            severe_deadline_packet_threshold,
            startup_retry_tick: tokio::time::interval(Duration::from_millis(120)),
            decode_drain_tick: tokio::time::interval(Duration::from_millis(4)),
            frame_event_count: 0,
            transport_event_count: 0,
            decode_tick_count: 0,
            startup_retry_tick_count: 0,
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
                _ = self.startup_retry_tick.tick() => {
                    self.startup_retry_tick_count = self.startup_retry_tick_count.saturating_add(1);
                    if self.startup_retry_tick_count == 1 || self.startup_retry_tick_count.is_power_of_two() {
                        crate::xbx_log_info!(
                            "[MediaSession] startup retry tick count={}",
                            self.startup_retry_tick_count
                        );
                    }
                    self.on_decode_drain_tick();
                    let _ = self
                        .recovery_input_tx
                        .send(RecoverySchedulerInput::StartupRetryTick);
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
        if matches!(
            decision,
            IngressDecision::WaitKeyframe | IngressDecision::Reconfigure
        ) {
            if self.frame_event_count == 1 || self.frame_event_count.is_power_of_two() {
                crate::xbx_log_warn!(
                    "[MediaSession] frame event triggered recovery decision={:?}",
                    decision
                );
            }
            let _ = self.recovery_input_tx.send(RecoverySchedulerInput::IngressSignal(
                crate::transport::rtc::recovery::signal::VideoIngressSignal::from_decision(
                    &decision,
                ),
            ));
        }
        self.on_decode_drain_tick();
    }

    async fn on_transport_observation(&mut self, observation: TransportObservation) {
        let signal = map_transport_observation_to_recovery_signal(
            observation,
            self.severe_deadline_packet_threshold,
        );
        let hint_now_ms = now_ms_f64();
        let diagnosis = signal.diagnose();
        if self.transport_event_count == 1 || self.transport_event_count.is_power_of_two() {
            crate::xbx_log_info!(
                "[MediaSession] transport observation diagnosis={}",
                diagnosis.label
            );
        }
        if self
            .observation
            .should_log_transport_hint(diagnosis.label, hint_now_ms)
        {
            crate::xbx_log_warn!(
                "[MediaSession] Transport escalation hint: {}",
                diagnosis.label
            );
            self.observation
                .record_transport_hint(diagnosis.label.to_string(), hint_now_ms);
        }
        let _ = self.recovery_input_tx.send(RecoverySchedulerInput::TransportSignal(signal));
    }
}

fn map_transport_observation_to_recovery_signal(
    observation: TransportObservation,
    severe_deadline_packet_threshold: usize,
) -> crate::transport::rtc::recovery::signal::VideoRecoverySignal {
    use crate::transport::rtc::recovery::signal::VideoRecoverySignal;

    match observation {
        TransportObservation::Admission(TransportAdmissionObservation::AwaitRecoveryKeyframe) => {
            VideoRecoverySignal::TransportAwaitRecoveryKeyframe
        }
        TransportObservation::Loss(TransportLossObservation::PacketLossDetected) => {
            VideoRecoverySignal::TransportSampleLoss
        }
        TransportObservation::Loss(TransportLossObservation::RecoveryKeyframeRequested) => {
            VideoRecoverySignal::TransportSampleLossBurst
        }
        TransportObservation::Loss(TransportLossObservation::AwaitRecoveryKeyframe) => {
            VideoRecoverySignal::TransportAwaitRecoveryKeyframe
        }
        TransportObservation::StreamIdleTimeout => VideoRecoverySignal::AdapterIdleTimeout,
        TransportObservation::StreamThinStall => VideoRecoverySignal::AdapterThinStream,
        TransportObservation::NackRecoveredLate => VideoRecoverySignal::TransportRecoveredLate,
        TransportObservation::NackDeadlineExpired { missing_packets } => {
            if usize::from(missing_packets) >= severe_deadline_packet_threshold {
                VideoRecoverySignal::TransportSevereDeadline
            } else {
                VideoRecoverySignal::TransportExpiredDeadline
            }
        }
    }
}

fn ingress_keyframe_cooldown_from_context(context: &MediaSessionContext) -> Duration {
    Duration::from_millis(
        context
            .runtime_config
            .webrtc
            .recovery
            .keyframe_request_stall_ms
            .max(250),
    )
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
