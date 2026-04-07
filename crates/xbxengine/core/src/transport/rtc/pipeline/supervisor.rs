use std::sync::{Arc, Mutex};

use tokio::runtime::Handle;

use crate::{
    media::video::render::renderer::XbxRenderState, transport::rtc::facts::TransportFact,
    transport::rtc::stream::adapter_types::VideoFramePipelineSources, XbxEngineMediaRuntimeStats,
    XbxEngineRuntimeConfig,
};

use super::session::{spawn_media_session, ActiveMediaSession, MediaSessionContext};

pub(crate) struct MediaSupervisorContext {
    pub(crate) runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    pub(crate) render_state: Arc<Mutex<XbxRenderState>>,
    pub(crate) transport_fact_sink: Arc<Mutex<Vec<TransportFact>>>,
    pub(crate) runtime_config: XbxEngineRuntimeConfig,
    pub(crate) local_decoder_reset_handle:
        Arc<Mutex<Option<Arc<crate::media::video::decode::actor::DecodeActorHandle>>>>,
}

pub(crate) fn spawn_media_supervisor(
    handle: Handle,
    mut frame_source_rx: tokio::sync::mpsc::Receiver<VideoFramePipelineSources>,
    context: MediaSupervisorContext,
) {
    handle.spawn(async move {
        crate::xbx_log_info!("[Supervisor] started waiting for video stream");
        let mut active_session: Option<ActiveMediaSession> = None;
        let mut mounted_count: u64 = 0;

        while let Some(frame_source) = frame_source_rx.recv().await {
            mounted_count = mounted_count.saturating_add(1);
            crate::xbx_log_info!("[Supervisor] mounting new video frame source");
            if mounted_count == 1 || mounted_count.is_power_of_two() {
                crate::xbx_log_info!("[Supervisor] frame source mount count={}", mounted_count);
            }

            if let Some(session) = active_session.take() {
                if let Ok(mut handle) = context.local_decoder_reset_handle.lock() {
                    *handle = None;
                }
                session.stop();
            }

            let session = spawn_media_session(
                frame_source,
                MediaSessionContext {
                    runtime_stats: context.runtime_stats.clone(),
                    render_state: context.render_state.clone(),
                    transport_fact_sink: context.transport_fact_sink.clone(),
                    runtime_config: context.runtime_config.clone(),
                },
            );
            if let Ok(mut handle) = context.local_decoder_reset_handle.lock() {
                *handle = Some(session.decode_handle());
            }
            active_session = Some(session);
        }

        if let Ok(mut handle) = context.local_decoder_reset_handle.lock() {
            *handle = None;
        }
    });
}
