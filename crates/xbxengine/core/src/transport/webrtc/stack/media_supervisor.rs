use std::sync::{Arc, Mutex};

use tokio::runtime::Handle;

use crate::{
    media::video::render::renderer::XbxRenderState, transport::adapter::FrameSource,
    transport::webrtc::data_channel::XbxDataChannelState, XbxEngineMediaRuntimeStats,
    XbxEnginePendingRuntimeRecoveryAction, XbxEngineRuntimeConfig,
};

use super::session::{spawn_media_session, ActiveMediaSession, MediaSessionContext};

pub(crate) struct MediaSupervisorContext {
    pub(crate) runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    pub(crate) pending_runtime_recovery_action:
        Arc<Mutex<Option<XbxEnginePendingRuntimeRecoveryAction>>>,
    pub(crate) data_channel_state: Arc<Mutex<XbxDataChannelState>>,
    pub(crate) render_state: Arc<Mutex<XbxRenderState>>,
    pub(crate) runtime_config: XbxEngineRuntimeConfig,
}

pub(crate) fn spawn_media_supervisor(
    handle: Handle,
    mut frame_source_rx: tokio::sync::mpsc::Receiver<Box<dyn FrameSource>>,
    context: MediaSupervisorContext,
) {
    handle.spawn(async move {
        crate::xbx_log_info!("[Supervisor] started waiting for video stream");
        let mut active_session: Option<ActiveMediaSession> = None;

        while let Some(frame_source) = frame_source_rx.recv().await {
            crate::xbx_log_info!("[Supervisor] mounting new video frame source");

            if let Some(session) = active_session.take() {
                session.stop();
            }

            active_session = Some(spawn_media_session(
                frame_source,
                MediaSessionContext {
                    runtime_stats: context.runtime_stats.clone(),
                    pending_runtime_recovery_action: context
                        .pending_runtime_recovery_action
                        .clone(),
                    data_channel_state: context.data_channel_state.clone(),
                    render_state: context.render_state.clone(),
                    runtime_config: context.runtime_config.clone(),
                },
            ));
        }
    });
}
