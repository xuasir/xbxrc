use std::sync::{Arc, Mutex};

use tokio::runtime::Handle;

use crate::{
    media::video::render::renderer::XbxRenderState,
    transport::rtc::media::adapter_types::VideoFramePipelineSources,
    transport::rtc::protocol::data_channel_state::XbxDataChannelState, XbxEngineMediaRuntimeStats,
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
                crate::xbx_log_info!(
                    "[Supervisor] frame source mount count={}",
                    mounted_count
                );
            }

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
