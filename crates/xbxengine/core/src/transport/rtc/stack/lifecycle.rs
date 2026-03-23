use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};

use xbxengine_protocol::XbxEngineTargetTypeDto;

use crate::api::backend::{
    XbxEngineMediaNegotiationRequest, XbxEngineMediaRuntimeStats,
    XbxEnginePendingRuntimeRecoveryAction,
};
use crate::media::video::render::renderer::XbxRenderState;
use crate::transport::rtc::connection::RtcConnectionService;
use crate::transport::rtc::facts::{ConnectionLifecycleStateFact, PeerFact, TransportFact};
use crate::transport::rtc::protocol::data_channel_state::XbxDataChannelState;
use crate::transport::rtc::session::actor::SessionActor;
use crate::transport::rtc::session::clock::SystemSessionClock;
use crate::transport::rtc::session::policy::RtcSessionPolicy;
use crate::transport::rtc::stream::audio::XbxRemoteAudioPlaybackSession;
use crate::transport::rtc::stream::RtcMediaService;
use crate::{XbxEngineRuntimeConfig, XbxEngineRuntimeError};

use super::input_loop::RtcInputStreamController;
use super::media_pipeline::{FrameSourceSender, RtcStackMediaPipelineBridge};
use super::transport_session::RtcTransportSessionBridge;

// 生命周期桥接只负责 reset/stop 编排，不承载协商与媒体算法。
pub(crate) struct RtcStackLifecycleBridge<'a> {
    media_runtime: &'a Arc<tokio::runtime::Runtime>,
    runtime_stats: &'a Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    pending_runtime_recovery_action: &'a Arc<Mutex<Option<XbxEnginePendingRuntimeRecoveryAction>>>,
    data_channel_state: &'a Arc<Mutex<XbxDataChannelState>>,
    render_state: &'a Arc<Mutex<XbxRenderState>>,
    runtime_config: &'a Arc<Mutex<XbxEngineRuntimeConfig>>,
    frame_source_tx: &'a Arc<Mutex<Option<FrameSourceSender>>>,
    audio_volume_bits: &'a Arc<AtomicU32>,
    audio_playback_session: &'a Arc<Mutex<Option<XbxRemoteAudioPlaybackSession>>>,
    connection: &'a Arc<Mutex<RtcConnectionService>>,
    media: &'a Arc<Mutex<RtcMediaService>>,
    transport_session: &'a Arc<Mutex<SessionActor<SystemSessionClock, RtcSessionPolicy>>>,
    transport_fact_sink: &'a Arc<Mutex<Vec<TransportFact>>>,
    input_stream: &'a mut RtcInputStreamController,
}

impl<'a> RtcStackLifecycleBridge<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        media_runtime: &'a Arc<tokio::runtime::Runtime>,
        runtime_stats: &'a Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        pending_runtime_recovery_action: &'a Arc<
            Mutex<Option<XbxEnginePendingRuntimeRecoveryAction>>,
        >,
        data_channel_state: &'a Arc<Mutex<XbxDataChannelState>>,
        render_state: &'a Arc<Mutex<XbxRenderState>>,
        runtime_config: &'a Arc<Mutex<XbxEngineRuntimeConfig>>,
        frame_source_tx: &'a Arc<Mutex<Option<FrameSourceSender>>>,
        audio_volume_bits: &'a Arc<AtomicU32>,
        audio_playback_session: &'a Arc<Mutex<Option<XbxRemoteAudioPlaybackSession>>>,
        connection: &'a Arc<Mutex<RtcConnectionService>>,
        media: &'a Arc<Mutex<RtcMediaService>>,
        transport_session: &'a Arc<Mutex<SessionActor<SystemSessionClock, RtcSessionPolicy>>>,
        transport_fact_sink: &'a Arc<Mutex<Vec<TransportFact>>>,
        input_stream: &'a mut RtcInputStreamController,
    ) -> Self {
        Self {
            media_runtime,
            runtime_stats,
            pending_runtime_recovery_action,
            data_channel_state,
            render_state,
            runtime_config,
            frame_source_tx,
            audio_volume_bits,
            audio_playback_session,
            connection,
            media,
            transport_session,
            transport_fact_sink,
            input_stream,
        }
    }

    pub(crate) fn sync_runtime_config(&self, runtime_config: XbxEngineRuntimeConfig) {
        if let Ok(mut config) = self.runtime_config.lock() {
            *config = runtime_config;
        }
        if let Ok(mut connection) = self.connection.lock() {
            connection.sync_runtime_config(
                self.runtime_config
                    .lock()
                    .ok()
                    .map(|config| config.webrtc.clone())
                    .unwrap_or_default(),
            );
        }
    }

    pub(crate) fn rebuild_peer_connection(
        &mut self,
        request: &XbxEngineMediaNegotiationRequest,
    ) -> Result<(), XbxEngineRuntimeError> {
        let _ = &self.media_runtime;
        self.transport_bridge().reset_transport_session();
        self.transport_bridge()
            .record_transport_fact(TransportFact::Peer(PeerFact::ConnectionStateChanged {
                state: ConnectionLifecycleStateFact::Connecting,
                observed_at_ms: crate::transport::rtc::stats::now_ms_f64(),
            }));
        self.reset_runtime_state(request.session.target_type.clone())?;
        self.media_pipeline_bridge().stop_audio_playback_session();
        if let Ok(mut media) = self.media.lock() {
            media.reset();
        }
        self.input_stream.reset_state();
        self.input_stream.ensure_running();
        self.media_pipeline_bridge().mount_legacy_frame_pipeline();
        if let Ok(mut connection) = self.connection.lock() {
            if let Ok(config) = self.runtime_config.lock() {
                connection.sync_runtime_config(config.webrtc.clone());
            }
        }
        self.connection
            .lock()
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcConnectionLockFailed"))?
            .rebuild(&request.session, self.runtime_stats)
    }

    pub(crate) fn stop(&mut self) {
        self.input_stream.stop();
        self.media_pipeline_bridge().stop_audio_playback_session();
        if let Ok(mut connection) = self.connection.lock() {
            connection.stop(self.runtime_stats);
        }
        self.clear_pending_runtime_recovery_action();
        if let Ok(mut render_state) = self.render_state.lock() {
            render_state.stop();
        }
        self.transport_bridge()
            .record_transport_fact(TransportFact::Peer(PeerFact::ConnectionStateChanged {
                state: ConnectionLifecycleStateFact::Closed,
                observed_at_ms: crate::transport::rtc::stats::now_ms_f64(),
            }));
    }

    fn reset_runtime_state(
        &self,
        session_target_type: XbxEngineTargetTypeDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        if let Ok(mut render_state) = self.render_state.lock() {
            render_state.reset()?;
        }
        self.clear_pending_runtime_recovery_action();
        if let Ok(mut data_channel_state) = self.data_channel_state.lock() {
            *data_channel_state = XbxDataChannelState::default();
        }
        if let Ok(mut stats) = self.runtime_stats.lock() {
            *stats = XbxEngineMediaRuntimeStats {
                session_target_type: Some(session_target_type),
                ..Default::default()
            };
        }
        Ok(())
    }

    fn clear_pending_runtime_recovery_action(&self) {
        if let Ok(mut pending_action) = self.pending_runtime_recovery_action.lock() {
            *pending_action = None;
        }
    }

    fn transport_bridge(&self) -> RtcTransportSessionBridge<'_> {
        RtcTransportSessionBridge::new(
            self.runtime_stats,
            self.pending_runtime_recovery_action,
            self.connection,
            self.media,
            self.transport_session,
            self.transport_fact_sink,
        )
    }

    fn media_pipeline_bridge(&self) -> RtcStackMediaPipelineBridge<'_> {
        RtcStackMediaPipelineBridge::new(
            self.media_runtime,
            self.runtime_stats,
            self.audio_volume_bits,
            self.audio_playback_session,
            self.media,
            self.frame_source_tx,
        )
    }
}
