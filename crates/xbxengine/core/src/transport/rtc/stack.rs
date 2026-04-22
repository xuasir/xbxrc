use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use ohmygamepad_protocol::OhMyGamepadRumbleRequestDto;
use xbxengine_protocol::{
    XbxEngineDisplayStateDto, XbxEngineIceCandidateDto, XbxEngineInputEventDto,
};

use crate::api::backend::{
    XbxEngineHostVideoFrameDropEvent, XbxEngineHostVideoPresentMetrics,
    XbxEngineMediaNegotiationRequest, XbxEngineMediaRuntimeStats,
    XbxEnginePendingRuntimeRecoveryAction, XbxEngineRenderFrame,
    XbxEngineVideoFrameDropObservation,
};
use crate::media::video::render::renderer::XbxRenderState;
use crate::transport::rtc::connection::{RtcConnectionService, VideoKeyframeRequestOutcome};
use crate::transport::rtc::facts::{CommandResultStatus, TransportCommand, TransportFact};
use crate::transport::rtc::pipeline::supervisor::{spawn_media_supervisor, MediaSupervisorContext};
use crate::transport::rtc::protocol::data_channel_state::{
    queue_keyboard_pointer_input, set_keyboard_pointer_enabled, XbxDataChannelState,
};
use crate::transport::rtc::session::actor::SessionActor;
use crate::transport::rtc::session::clock::SystemSessionClock;
use crate::transport::rtc::session::policy::RtcSessionPolicy;
use crate::transport::rtc::stream::audio::XbxRemoteAudioPlaybackSession;
use crate::transport::rtc::stream::RtcMediaService;
use crate::{XbxEngineRuntimeConfig, XbxEngineRuntimeError};

mod input_loop;
mod lifecycle;
mod media_pipeline;
mod negotiation;
mod runtime_port;
mod runtime_stats;
mod transport_session;
use self::input_loop::RtcInputStreamController;
use self::lifecycle::RtcStackLifecycleBridge;
use self::negotiation::RtcStackNegotiationBridge;
use self::runtime_port::{LocalDecoderResetPolicyState, RtcStackRuntimePort};
use self::transport_session::RtcTransportSessionBridge;
#[cfg(test)]
pub(crate) use self::transport_session::RtcTransportSessionBridge as TestRtcTransportSessionBridge;

pub(crate) trait XbxMediaStackPort: Send {
    fn sync_runtime_config(&mut self, runtime_config: XbxEngineRuntimeConfig);
    fn rebuild_peer_connection(
        &mut self,
        request: &XbxEngineMediaNegotiationRequest,
    ) -> Result<(), XbxEngineRuntimeError>;
    fn create_offer(&self) -> Result<String, XbxEngineRuntimeError>;
    fn apply_remote_description(
        &self,
        answer_sdp: &str,
        remote_candidates: &[XbxEngineIceCandidateDto],
    ) -> Result<(), XbxEngineRuntimeError>;
    fn add_remote_ice_candidates(
        &self,
        remote_candidates: &[XbxEngineIceCandidateDto],
    ) -> Result<(), XbxEngineRuntimeError>;
    fn apply_display_state(
        &mut self,
        state: XbxEngineDisplayStateDto,
    ) -> Result<(), XbxEngineRuntimeError>;
    fn local_candidates_snapshot(&self) -> Vec<XbxEngineIceCandidateDto>;
    fn local_ice_gathering_complete(&self) -> bool;
    fn snapshot_runtime_stats(&self) -> XbxEngineMediaRuntimeStats;
    fn take_pending_gamepad_rumble_requests(&mut self) -> Vec<OhMyGamepadRumbleRequestDto>;
    fn take_pending_runtime_recovery_action(
        &mut self,
    ) -> Option<XbxEnginePendingRuntimeRecoveryAction>;
    fn drain_pending_render_frames(&mut self) -> Vec<XbxEngineRenderFrame>;
    fn record_video_frame_drop(&mut self, observation: XbxEngineVideoFrameDropObservation);
    fn set_audio_volume(&mut self, value: f32);
    fn set_microphone_capturing(&mut self, capturing: bool) -> Result<(), XbxEngineRuntimeError>;
    fn set_keyboard_pointer_enabled(&mut self, enabled: bool) -> Result<(), XbxEngineRuntimeError>;
    fn push_keyboard_pointer_input(
        &mut self,
        event: XbxEngineInputEventDto,
    ) -> Result<(), XbxEngineRuntimeError>;
    fn request_video_pli(&mut self) -> Result<(), XbxEngineRuntimeError>;
    fn request_decoder_reset(&mut self) -> Result<(), XbxEngineRuntimeError>;
    fn update_host_video_timing(
        &mut self,
        host_display_interval_ms: Option<f64>,
        host_frame_age_budget_ms: Option<f64>,
    );
    fn update_host_video_present_metrics(&mut self, metrics: XbxEngineHostVideoPresentMetrics);
    fn record_host_video_frame_drop(&mut self, event: XbxEngineHostVideoFrameDropEvent);
    fn stop(&mut self);
}

pub(crate) struct XbxActiveMediaStack {
    media_runtime: Arc<tokio::runtime::Runtime>,
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    pending_runtime_recovery_action: Arc<Mutex<Option<XbxEnginePendingRuntimeRecoveryAction>>>,
    data_channel_state: Arc<Mutex<XbxDataChannelState>>,
    render_state: Arc<Mutex<XbxRenderState>>,
    runtime_config: Arc<Mutex<XbxEngineRuntimeConfig>>,
    last_request: Arc<Mutex<Option<XbxEngineMediaNegotiationRequest>>>,
    frame_source_tx: Arc<
        Mutex<
            Option<
                tokio::sync::mpsc::Sender<
                    crate::transport::rtc::stream::adapter_types::VideoFramePipelineSources,
                >,
            >,
        >,
    >,
    audio_volume_bits: Arc<AtomicU32>,
    audio_playback_session: Arc<Mutex<Option<XbxRemoteAudioPlaybackSession>>>,
    connection: Arc<Mutex<RtcConnectionService>>,
    media: Arc<Mutex<RtcMediaService>>,
    local_decoder_reset_handle:
        Arc<Mutex<Option<Arc<crate::media::video::decode::actor::DecodeActorHandle>>>>,
    local_decoder_reset_policy: Arc<Mutex<LocalDecoderResetPolicyState>>,
    transport_session: Arc<Mutex<SessionActor<SystemSessionClock, RtcSessionPolicy>>>,
    transport_fact_sink: Arc<Mutex<Vec<TransportFact>>>,
    input_stream: RtcInputStreamController,
}

impl XbxActiveMediaStack {
    fn transport_bridge(&self) -> RtcTransportSessionBridge<'_> {
        RtcTransportSessionBridge::new(
            &self.runtime_stats,
            &self.runtime_config,
            &self.pending_runtime_recovery_action,
            &self.connection,
            &self.media,
            &self.local_decoder_reset_handle,
            &self.transport_session,
            &self.transport_fact_sink,
        )
    }

    fn negotiation_bridge(&self) -> RtcStackNegotiationBridge<'_> {
        RtcStackNegotiationBridge::new(
            &self.runtime_config,
            &self.last_request,
            &self.runtime_stats,
            &self.connection,
            &self.media,
        )
    }

    fn lifecycle_bridge(&mut self) -> RtcStackLifecycleBridge<'_> {
        RtcStackLifecycleBridge::new(
            &self.media_runtime,
            &self.runtime_stats,
            &self.pending_runtime_recovery_action,
            &self.data_channel_state,
            &self.render_state,
            &self.runtime_config,
            &self.frame_source_tx,
            &self.audio_volume_bits,
            &self.audio_playback_session,
            &self.connection,
            &self.media,
            &self.local_decoder_reset_handle,
            &self.transport_session,
            &self.transport_fact_sink,
            &mut self.input_stream,
        )
    }

    fn runtime_port(&self) -> RtcStackRuntimePort<'_> {
        RtcStackRuntimePort::new(
            &self.runtime_stats,
            &self.render_state,
            &self.media,
            &self.local_decoder_reset_handle,
            &self.local_decoder_reset_policy,
        )
    }

    pub(crate) fn new(runtime_config: XbxEngineRuntimeConfig) -> Self {
        let runtime_config_for_supervisor = runtime_config.clone();
        let runtime_config = Arc::new(Mutex::new(runtime_config));
        let media_runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                // tokio worker 默认栈较小；当链路进入异常自旋/深层 Future 链时容易触发 stack overflow 直接 abort。
                // 先提升栈上限以保证可观测性与稳定性，根因仍需要通过 trace/采样进一步定位。
                .thread_stack_size(8 * 1024 * 1024)
                .enable_all()
                .build()
                .expect("build rtc media runtime"),
        );
        let (frame_source_tx, frame_source_rx) = tokio::sync::mpsc::channel::<
            crate::transport::rtc::stream::adapter_types::VideoFramePipelineSources,
        >(1);
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let data_channel_state = Arc::new(Mutex::new(XbxDataChannelState::default()));
        let render_state = Arc::new(Mutex::new(XbxRenderState::default()));
        let transport_fact_sink = Arc::new(Mutex::new(Vec::new()));
        let local_decoder_reset_handle = Arc::new(Mutex::new(None));
        spawn_media_supervisor(
            media_runtime.handle().clone(),
            frame_source_rx,
            MediaSupervisorContext {
                runtime_stats: runtime_stats.clone(),
                render_state: render_state.clone(),
                transport_fact_sink: transport_fact_sink.clone(),
                runtime_config: runtime_config_for_supervisor,
                local_decoder_reset_handle: local_decoder_reset_handle.clone(),
            },
        );
        let connection = Arc::new(Mutex::new(RtcConnectionService::default()));
        let input_stream = RtcInputStreamController::new(
            media_runtime.clone(),
            connection.clone(),
            runtime_stats.clone(),
            data_channel_state.clone(),
        );
        let mut stack = Self {
            media_runtime,
            runtime_stats: runtime_stats.clone(),
            pending_runtime_recovery_action,
            data_channel_state,
            render_state,
            runtime_config: runtime_config.clone(),
            last_request: Arc::new(Mutex::new(None)),
            frame_source_tx: Arc::new(Mutex::new(Some(frame_source_tx))),
            audio_volume_bits: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            audio_playback_session: Arc::new(Mutex::new(None)),
            connection,
            media: Arc::new(Mutex::new(RtcMediaService::default())),
            local_decoder_reset_handle,
            local_decoder_reset_policy: Arc::new(Mutex::new(
                LocalDecoderResetPolicyState::default(),
            )),
            transport_session: Arc::new(Mutex::new(SessionActor::new(
                SystemSessionClock,
                RtcSessionPolicy::new(runtime_config.clone(), runtime_stats.clone()),
            ))),
            transport_fact_sink,
            input_stream,
        };
        stack.input_stream.ensure_running();
        stack
    }

    fn pump_connection_and_media_ingress(&self) {
        self.transport_bridge().pump_connection_and_media_ingress();
    }

    fn record_transport_command_status(
        &self,
        command: TransportCommand,
        status: CommandResultStatus,
    ) {
        self.transport_bridge()
            .record_transport_command_status(command, status);
    }
}

impl XbxMediaStackPort for XbxActiveMediaStack {
    fn sync_runtime_config(&mut self, runtime_config: XbxEngineRuntimeConfig) {
        self.lifecycle_bridge().sync_runtime_config(runtime_config);
    }

    fn rebuild_peer_connection(
        &mut self,
        request: &XbxEngineMediaNegotiationRequest,
    ) -> Result<(), XbxEngineRuntimeError> {
        if let Ok(mut last_request) = self.last_request.lock() {
            *last_request = Some(request.clone());
        }
        self.lifecycle_bridge().rebuild_peer_connection(request)
    }

    fn create_offer(&self) -> Result<String, XbxEngineRuntimeError> {
        self.negotiation_bridge().create_offer()
    }

    fn apply_remote_description(
        &self,
        answer_sdp: &str,
        remote_candidates: &[XbxEngineIceCandidateDto],
    ) -> Result<(), XbxEngineRuntimeError> {
        self.negotiation_bridge()
            .apply_remote_description(answer_sdp, remote_candidates)
    }

    fn add_remote_ice_candidates(
        &self,
        remote_candidates: &[XbxEngineIceCandidateDto],
    ) -> Result<(), XbxEngineRuntimeError> {
        self.negotiation_bridge()
            .add_remote_ice_candidates(remote_candidates)
    }

    fn apply_display_state(
        &mut self,
        state: XbxEngineDisplayStateDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.runtime_port().apply_display_state(state)
    }

    fn local_candidates_snapshot(&self) -> Vec<XbxEngineIceCandidateDto> {
        self.pump_connection_and_media_ingress();
        self.negotiation_bridge().local_candidates_snapshot()
    }

    fn local_ice_gathering_complete(&self) -> bool {
        self.pump_connection_and_media_ingress();
        self.negotiation_bridge().local_ice_gathering_complete()
    }

    fn snapshot_runtime_stats(&self) -> XbxEngineMediaRuntimeStats {
        self.pump_connection_and_media_ingress();
        crate::xbx_log_debug!("[xbxengine][rtc-stack] snapshot_runtime_stats after pump");
        self.runtime_port().snapshot_runtime_stats()
    }

    fn take_pending_gamepad_rumble_requests(&mut self) -> Vec<OhMyGamepadRumbleRequestDto> {
        self.connection
            .lock()
            .ok()
            .map(|mut connection| connection.take_pending_gamepad_rumble_requests())
            .unwrap_or_default()
    }

    fn take_pending_runtime_recovery_action(
        &mut self,
    ) -> Option<XbxEnginePendingRuntimeRecoveryAction> {
        self.pending_runtime_recovery_action
            .lock()
            .ok()
            .and_then(|mut action| action.take())
    }

    fn drain_pending_render_frames(&mut self) -> Vec<XbxEngineRenderFrame> {
        self.runtime_port().drain_pending_render_frames()
    }

    fn record_video_frame_drop(&mut self, observation: XbxEngineVideoFrameDropObservation) {
        self.runtime_port().record_video_frame_drop(observation);
    }

    fn set_audio_volume(&mut self, value: f32) {
        self.audio_volume_bits
            .store(value.to_bits(), Ordering::Relaxed);
    }

    fn set_microphone_capturing(&mut self, _capturing: bool) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }

    fn set_keyboard_pointer_enabled(&mut self, enabled: bool) -> Result<(), XbxEngineRuntimeError> {
        self.connection
            .lock()
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcConnectionLockFailed"))?
            .set_keyboard_pointer_enabled(enabled);
        set_keyboard_pointer_enabled(&self.data_channel_state, enabled);
        Ok(())
    }

    fn push_keyboard_pointer_input(
        &mut self,
        event: XbxEngineInputEventDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        queue_keyboard_pointer_input(&self.data_channel_state, event);
        Ok(())
    }

    fn request_video_pli(&mut self) -> Result<(), XbxEngineRuntimeError> {
        let result = self
            .connection
            .lock()
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcConnectionLockFailed"))?
            .request_video_pli_with_outcome(&self.runtime_stats);
        let status = match &result {
            Ok(
                VideoKeyframeRequestOutcome::RequestedPli
                | VideoKeyframeRequestOutcome::RequestedFir,
            ) => CommandResultStatus::Succeeded,
            Ok(VideoKeyframeRequestOutcome::Suppressed) => CommandResultStatus::Deferred {
                reason: "sameFamilyTransportStageCoalesced".to_string(),
            },
            Ok(VideoKeyframeRequestOutcome::FeedbackTargetPending) => {
                CommandResultStatus::Deferred {
                    reason: "familyDeferred:videoRtcpFeedbackTargetPending".to_string(),
                }
            }
            Err(error) => CommandResultStatus::Failed {
                error: error.to_string(),
            },
        };
        self.record_transport_command_status(
            TransportCommand::RequestPli {
                reason: "stack.manualRequest".to_string(),
                observation_id: 0,
            },
            status,
        );
        result.map(|_| ())
    }

    fn request_decoder_reset(&mut self) -> Result<(), XbxEngineRuntimeError> {
        self.transport_bridge()
            .request_local_decoder_reset("stack.manualRequest".to_string())
    }

    fn update_host_video_timing(
        &mut self,
        host_display_interval_ms: Option<f64>,
        host_frame_age_budget_ms: Option<f64>,
    ) {
        self.runtime_port()
            .update_host_video_timing(host_display_interval_ms, host_frame_age_budget_ms);
    }

    fn update_host_video_present_metrics(&mut self, metrics: XbxEngineHostVideoPresentMetrics) {
        self.runtime_port()
            .update_host_video_present_metrics(metrics);
    }

    fn record_host_video_frame_drop(&mut self, event: XbxEngineHostVideoFrameDropEvent) {
        self.runtime_port().record_host_video_frame_drop(event);
    }

    fn stop(&mut self) {
        self.lifecycle_bridge().stop();
    }
}

#[cfg(test)]
mod tests {
    use super::runtime_stats::merge_media_snapshot_into_runtime_stats;
    use crate::api::backend::XbxEngineMediaRuntimeStats;
    use crate::transport::rtc::stream::runtime_state::RtcMediaIngressSnapshot;
    use xbxengine_protocol::XbxEngineTransportStateDto;

    #[test]
    fn media_snapshot_promotes_track_status_to_remote_track_attached_when_video_exists() {
        let mut stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            inbound_audio_bytes_total: 64,
            latest_video_track_status: Some(crate::XbxEngineVideoTrackStatus {
                state: "audioOnly".to_string(),
                video_width: None,
                video_height: None,
                mime_type: Some("video/h264".to_string()),
                transport_state: XbxEngineTransportStateDto::Connected,
                video_bytes_total: 0,
                video_packet_count_total: 0,
                audio_bytes_total: 64,
                observed_at_ms: 10.0,
            }),
            latest_video_stream_width: Some(1920),
            latest_video_stream_height: Some(1080),
            ..XbxEngineMediaRuntimeStats::default()
        };
        let media_snapshot = RtcMediaIngressSnapshot {
            inbound_primary_video_count: 12,
            inbound_primary_video_bytes: 12_000,
            inbound_repair_video_count: 3,
            inbound_repair_video_bytes: 600,
            inbound_audio_bytes: 128,
            ..RtcMediaIngressSnapshot::default()
        };

        merge_media_snapshot_into_runtime_stats(&mut stats, &media_snapshot, 123.0);

        let first_status = stats
            .latest_video_track_status
            .as_ref()
            .expect("video track status should exist after first merge");
        assert_eq!(first_status.state, "primaryVideoRtpStarted");
        assert_eq!(first_status.video_width, Some(1920));
        assert_eq!(first_status.video_height, Some(1080));
        assert_eq!(first_status.video_bytes_total, 12_000);
        assert_eq!(first_status.video_packet_count_total, 12);
        assert_eq!(first_status.audio_bytes_total, 128);

        merge_media_snapshot_into_runtime_stats(&mut stats, &media_snapshot, 124.0);

        assert_eq!(stats.inbound_video_packet_count_total, 15);
        assert_eq!(stats.inbound_video_bytes_total, 12_600);
        assert_eq!(stats.inbound_primary_video_bytes_total, 12_000);
        assert_eq!(stats.inbound_audio_bytes_total, 128);
        let status = stats
            .latest_video_track_status
            .expect("video track status should exist");
        assert_eq!(status.state, "remoteTrackAttached");
        assert_eq!(status.video_width, Some(1920));
        assert_eq!(status.video_height, Some(1080));
        assert_eq!(status.video_bytes_total, 12_600);
        assert_eq!(status.video_packet_count_total, 15);
        assert_eq!(status.audio_bytes_total, 128);
    }

    #[test]
    fn media_snapshot_track_state_machine_flows_audio_then_video_then_attached() {
        let mut stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            ..XbxEngineMediaRuntimeStats::default()
        };

        merge_media_snapshot_into_runtime_stats(
            &mut stats,
            &RtcMediaIngressSnapshot {
                inbound_audio_bytes: 96,
                ..RtcMediaIngressSnapshot::default()
            },
            10.0,
        );
        assert_eq!(
            stats
                .latest_video_track_status
                .as_ref()
                .map(|status| status.state.as_str()),
            Some("audioOnly")
        );

        merge_media_snapshot_into_runtime_stats(
            &mut stats,
            &RtcMediaIngressSnapshot {
                inbound_primary_video_count: 3,
                inbound_primary_video_bytes: 3_000,
                inbound_audio_bytes: 96,
                ..RtcMediaIngressSnapshot::default()
            },
            20.0,
        );
        assert_eq!(
            stats
                .latest_video_track_status
                .as_ref()
                .map(|status| status.state.as_str()),
            Some("primaryVideoRtpStarted")
        );

        merge_media_snapshot_into_runtime_stats(
            &mut stats,
            &RtcMediaIngressSnapshot {
                inbound_primary_video_count: 6,
                inbound_primary_video_bytes: 6_000,
                inbound_audio_bytes: 96,
                ..RtcMediaIngressSnapshot::default()
            },
            30.0,
        );
        let status = stats
            .latest_video_track_status
            .as_ref()
            .expect("video track status should exist");
        assert_eq!(status.state, "remoteTrackAttached");
        assert_eq!(status.video_packet_count_total, 6);
        assert_eq!(status.video_bytes_total, 6_000);
    }
}
