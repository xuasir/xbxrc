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
    local_decoder_reset_handle:
        &'a Arc<Mutex<Option<Arc<crate::media::video::decode::actor::DecodeActorHandle>>>>,
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
        local_decoder_reset_handle: &'a Arc<
            Mutex<Option<Arc<crate::media::video::decode::actor::DecodeActorHandle>>>,
        >,
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
            local_decoder_reset_handle,
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
        crate::runtime_stats_sink::RuntimeStatsSink::new(self.runtime_stats.clone())
            .record_video_ingress_close_intent(
                crate::transport::rtc::stats::now_ms_f64(),
                "rebuildPeerConnection",
            );
        self.media_pipeline_bridge().stop_audio_playback_session();
        if let Ok(mut media) = self.media.lock() {
            media.reset();
        }
        self.reset_runtime_state(request.session.target_type.clone())?;
        self.input_stream.reset_state();
        self.input_stream.ensure_running();
        self.media_pipeline_bridge().mount_primary_frame_pipeline();
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
        crate::runtime_stats_sink::RuntimeStatsSink::new(self.runtime_stats.clone())
            .record_video_ingress_close_intent(
                crate::transport::rtc::stats::now_ms_f64(),
                "stackStop",
            );
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
            *stats = reset_runtime_stats_for_new_session(&stats, session_target_type);
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
            self.runtime_config,
            self.pending_runtime_recovery_action,
            self.connection,
            self.media,
            self.local_decoder_reset_handle,
            self.transport_session,
            self.transport_fact_sink,
        )
    }

    fn media_pipeline_bridge(&self) -> RtcStackMediaPipelineBridge<'_> {
        RtcStackMediaPipelineBridge::new(
            self.media_runtime,
            self.runtime_stats,
            self.runtime_config,
            self.audio_volume_bits,
            self.audio_playback_session,
            self.connection,
            self.media,
            self.frame_source_tx,
        )
    }
}

fn reset_runtime_stats_for_new_session(
    previous: &XbxEngineMediaRuntimeStats,
    session_target_type: XbxEngineTargetTypeDto,
) -> XbxEngineMediaRuntimeStats {
    let mut next = XbxEngineMediaRuntimeStats {
        session_target_type: Some(session_target_type),
        ..Default::default()
    };
    next.latest_video_ingress_close_intent_cause =
        previous.latest_video_ingress_close_intent_cause.clone();
    next.latest_video_ingress_close_intent_observed_at_ms =
        previous.latest_video_ingress_close_intent_observed_at_ms;
    next
}

#[cfg(test)]
mod tests {
    use super::reset_runtime_stats_for_new_session;
    use crate::api::backend::XbxEngineMediaRuntimeStats;
    use xbxengine_protocol::XbxEngineTargetTypeDto;

    #[test]
    fn reset_runtime_stats_preserves_video_ingress_close_intent() {
        let mut previous = XbxEngineMediaRuntimeStats::default();
        previous.latest_video_ingress_close_intent_cause =
            Some("rebuildPeerConnection".to_string());
        previous.latest_video_ingress_close_intent_observed_at_ms = Some(1234.0);
        previous.latest_observation_label = Some("oldLabel".to_string());
        previous.transport_recovery_epoch = 99;

        let next = reset_runtime_stats_for_new_session(&previous, XbxEngineTargetTypeDto::Cloud);

        assert_eq!(
            next.latest_video_ingress_close_intent_cause.as_deref(),
            Some("rebuildPeerConnection")
        );
        assert_eq!(
            next.latest_video_ingress_close_intent_observed_at_ms,
            Some(1234.0)
        );
        assert_eq!(next.latest_observation_label, None);
        assert_eq!(next.transport_recovery_epoch, 0);
        assert_eq!(
            next.session_target_type,
            Some(XbxEngineTargetTypeDto::Cloud)
        );
    }
}
