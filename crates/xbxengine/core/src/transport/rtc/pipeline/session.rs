use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;

use crate::{
    api::backend::XbxHostRenderFramePush,
    media::video::render::renderer::XbxRenderState,
    media::video::types::AssembledVideoFrame,
    transport::rtc::facts::TransportFact,
    transport::rtc::stream::adapter_types::{TransportObservation, VideoFramePipelineSources},
    XbxEngineMediaRuntimeStats, XbxEngineRuntimeConfig,
};

use super::session_loop::{spawn_media_session_loop, MediaSessionLoopConfig};

#[derive(Clone)]
pub(super) struct MediaSessionContext {
    pub(super) runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    pub(super) render_state: Arc<Mutex<XbxRenderState>>,
    pub(super) transport_fact_sink: Arc<Mutex<Vec<TransportFact>>>,
    pub(super) runtime_config: XbxEngineRuntimeConfig,
    pub(super) host_render_frame_push: Arc<Mutex<Option<Arc<dyn XbxHostRenderFramePush>>>>,
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
    pub(super) fn decode_handle(
        &self,
    ) -> Arc<crate::media::video::decode::actor::DecodeActorHandle> {
        self.decode.clone()
    }

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
            context.host_render_frame_push.clone(),
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

    let task = spawn_media_session_loop(
        frame_rx,
        transport_observation_rx,
        decode_handle.clone(),
        context.runtime_stats.clone(),
        context.transport_fact_sink.clone(),
        MediaSessionLoopConfig {
            jitter_buffer_min_delay,
            jitter_buffer_max_delay,
            late_frame_drop_threshold_ms: video_pipeline_config.late_frame_drop_threshold_ms,
            backlog_drop_threshold_packets: video_pipeline_config.backlog_drop_threshold_packets,
            severe_deadline_packet_threshold,
        },
    );

    ActiveMediaSession {
        decode: decode_handle,
        pacer: pacer_handle,
        renderer: renderer_handle,
        frame_source_task,
        transport_observation_task,
        task,
    }
}
