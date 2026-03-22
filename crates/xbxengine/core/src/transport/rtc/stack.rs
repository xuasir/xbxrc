use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use ohmygamepad_host::GamepadRuntimeHost;
use ohmygamepad_protocol::{LogicalPadSnapshotDto, OhMyGamepadRouteTargetDto};
use tokio::task::JoinHandle;
use tokio::time::{interval, Duration};
use xbxengine_protocol::{
    XbxEngineDisplayStateDto, XbxEngineIceCandidateDto, XbxEngineInputEventDto,
};

use crate::api::backend::{
    XbxEngineMediaNegotiationRequest, XbxEngineMediaRuntimeStats,
    XbxEnginePendingRuntimeRecoveryAction, XbxEngineRenderFrame,
};
use crate::media::video::render::renderer::XbxRenderState;
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::connection::RtcConnectionService;
use crate::transport::rtc::executor::peer::stage_reconnect_candidate;
use crate::transport::rtc::facts::{
    CommandResultFact, CommandResultStatus, ConnectionLifecycleStateFact, PeerFact, TimerFact,
    TransportCommand, TransportFact,
};
use crate::transport::rtc::media::{build_rtc_video_frame_source, RtcMediaService};
use crate::transport::rtc::pipeline::supervisor::{spawn_media_supervisor, MediaSupervisorContext};
use crate::transport::rtc::protocol::data_channel_state::{
    build_input_stream_packet, build_metadata_frame, drain_pending_input_frames,
    queue_keyboard_pointer_input, set_keyboard_pointer_enabled, XbxDataChannelState,
    STREAM_INPUT_IDLE_GAMEPAD_KEEPALIVE_MS,
};
use crate::transport::rtc::sdp::policy::{summarize_sdp, validate_local_offer_sdp};
use crate::transport::rtc::sdp::{
    adapt_local_offer, adapt_remote_answer, normalize_remote_candidate, RtcSdpContext,
};
use crate::transport::rtc::session::actor::SessionActor;
use crate::transport::rtc::session::clock::SystemSessionClock;
use crate::transport::rtc::session::policy::RtcSessionPolicy;
use crate::{XbxEngineRuntimeConfig, XbxEngineRuntimeError};

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
    fn take_pending_runtime_recovery_action(
        &mut self,
    ) -> Option<XbxEnginePendingRuntimeRecoveryAction>;
    fn take_latest_render_frame(&mut self) -> Option<XbxEngineRenderFrame>;
    fn set_audio_volume(&mut self, value: f32);
    fn set_microphone_capturing(&mut self, capturing: bool) -> Result<(), XbxEngineRuntimeError>;
    fn set_keyboard_pointer_enabled(&mut self, enabled: bool) -> Result<(), XbxEngineRuntimeError>;
    fn push_keyboard_pointer_input(
        &mut self,
        event: XbxEngineInputEventDto,
    ) -> Result<(), XbxEngineRuntimeError>;
    fn request_video_keyframe(&mut self) -> Result<(), XbxEngineRuntimeError>;
    fn request_decoder_reset(&mut self) -> Result<(), XbxEngineRuntimeError>;
    fn update_host_video_timing(
        &mut self,
        host_display_interval_ms: Option<f64>,
        host_frame_age_budget_ms: Option<f64>,
    );
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
                    crate::transport::rtc::media::adapter_types::VideoFramePipelineSources,
                >,
            >,
        >,
    >,
    connection: Arc<Mutex<RtcConnectionService>>,
    media: Arc<Mutex<RtcMediaService>>,
    transport_session: Arc<Mutex<SessionActor<SystemSessionClock, RtcSessionPolicy>>>,
    transport_fact_sink: Arc<Mutex<Vec<TransportFact>>>,
    input_stream_state: Arc<Mutex<RtcInputStreamState>>,
    input_loop_stop: Arc<AtomicBool>,
    input_loop_task: Option<JoinHandle<()>>,
}

#[derive(Clone, Debug)]
struct RtcInputStreamState {
    input_sequence: u32,
    last_input_packet_sent_at_ms: f64,
    last_gamepad_sample_count: usize,
    last_gamepad_sample_signature: [u64; 4],
    gamepad_sample_signature: [u64; 4],
    last_metadata_frame_seq: u64,
}

impl Default for RtcInputStreamState {
    fn default() -> Self {
        Self {
            input_sequence: 1,
            last_input_packet_sent_at_ms: 0.0,
            last_gamepad_sample_count: 0,
            last_gamepad_sample_signature: [0; 4],
            gamepad_sample_signature: [0; 4],
            last_metadata_frame_seq: 0,
        }
    }
}

const RTC_INPUT_STREAM_POLL_INTERVAL_MS: u64 = 8;

impl XbxActiveMediaStack {
    pub(crate) fn new(runtime_config: XbxEngineRuntimeConfig) -> Self {
        let media_runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("build rtc media runtime"),
        );
        let (frame_source_tx, frame_source_rx) = tokio::sync::mpsc::channel::<
            crate::transport::rtc::media::adapter_types::VideoFramePipelineSources,
        >(1);
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let data_channel_state = Arc::new(Mutex::new(XbxDataChannelState::default()));
        let render_state = Arc::new(Mutex::new(XbxRenderState::default()));
        let transport_fact_sink = Arc::new(Mutex::new(Vec::new()));
        spawn_media_supervisor(
            media_runtime.handle().clone(),
            frame_source_rx,
            MediaSupervisorContext {
                runtime_stats: runtime_stats.clone(),
                render_state: render_state.clone(),
                transport_fact_sink: transport_fact_sink.clone(),
                runtime_config: runtime_config.clone(),
            },
        );
        let connection = Arc::new(Mutex::new(RtcConnectionService::default()));
        let mut stack = Self {
            media_runtime,
            runtime_stats,
            pending_runtime_recovery_action,
            data_channel_state,
            render_state,
            runtime_config: Arc::new(Mutex::new(runtime_config)),
            last_request: Arc::new(Mutex::new(None)),
            frame_source_tx: Arc::new(Mutex::new(Some(frame_source_tx))),
            connection,
            media: Arc::new(Mutex::new(RtcMediaService::default())),
            transport_session: Arc::new(Mutex::new(SessionActor::new(
                SystemSessionClock,
                RtcSessionPolicy::default(),
            ))),
            transport_fact_sink,
            input_stream_state: Arc::new(Mutex::new(RtcInputStreamState::default())),
            input_loop_stop: Arc::new(AtomicBool::new(false)),
            input_loop_task: None,
        };
        stack.ensure_input_loop_running();
        stack
    }

    fn ensure_input_loop_running(&mut self) {
        if self
            .input_loop_task
            .as_ref()
            .is_some_and(|task| !task.is_finished())
        {
            return;
        }
        self.input_loop_stop.store(false, Ordering::Relaxed);
        let connection = self.connection.clone();
        let runtime_stats = self.runtime_stats.clone();
        let data_channel_state = self.data_channel_state.clone();
        let input_stream_state = self.input_stream_state.clone();
        let stop = self.input_loop_stop.clone();
        let task = self.media_runtime.handle().spawn(async move {
            let mut ticker = interval(Duration::from_millis(RTC_INPUT_STREAM_POLL_INTERVAL_MS));
            ticker.tick().await;
            loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                ticker.tick().await;
                Self::pump_input_stream_once(
                    &connection,
                    &runtime_stats,
                    &data_channel_state,
                    &input_stream_state,
                );
            }
        });
        self.input_loop_task = Some(task);
    }

    fn mount_legacy_frame_pipeline(&self) {
        struct DummyRtcpPort;
        impl crate::transport::rtc::media::sink::RtcRtcpSendPort for DummyRtcpPort {
            fn send_rtcp(&self, _buf: &[u8]) {}
        }
        let (sink, frame_sources) = build_rtc_video_frame_source(
            8192,
            Arc::new(DummyRtcpPort),
            self.runtime_stats.clone(),
            300,
            Duration::from_millis(0),
            Duration::from_millis(50),
            Duration::from_millis(500),
            crate::transport::rtc::media::nack_scheduler::NackSchedulerConfig {
                max_age_ms: 200,
                frame_deadline_ms: 120,
                burst_count: 4,
                retry_interval_ms: 40,
                max_retry_count: 3,
            },
        );
        if let Ok(mut media) = self.media.lock() {
            media.set_sink(sink);
        }
        let send_result = self
            .frame_source_tx
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().cloned())
            .map(|sender| sender.blocking_send(frame_sources));
        match send_result {
            Some(Ok(())) => {
                crate::xbx_log_info!(
                    "[xbxengine][rtc] legacy frame pipeline mounted and handed to supervisor"
                );
                RuntimeStatsSink::new(self.runtime_stats.clone()).update(|stats| {
                    stats.latest_observation_label =
                        Some("rtcLegacyFramePipelineMounted".to_string());
                    stats.latest_observation_summary =
                        Some("phase1 rtc mounted legacy sample-builder frame pipeline".to_string());
                });
            }
            Some(Err(err)) => {
                crate::xbx_log_info!(
                    "[xbxengine][rtc] legacy frame pipeline mount failed err={err}"
                );
                RuntimeStatsSink::new(self.runtime_stats.clone()).update(|stats| {
                    stats.latest_observation_label =
                        Some("rtcLegacyFramePipelineMountFailed".to_string());
                    stats.latest_observation_summary = Some(format!(
                        "phase1 rtc mount legacy frame pipeline failed err={err}"
                    ));
                });
            }
            None => {
                crate::xbx_log_info!("[xbxengine][rtc] legacy frame pipeline sender unavailable");
                RuntimeStatsSink::new(self.runtime_stats.clone()).update(|stats| {
                    stats.latest_observation_label =
                        Some("rtcLegacyFramePipelineSenderMissing".to_string());
                    stats.latest_observation_summary =
                        Some("phase1 rtc frame source sender unavailable".to_string());
                });
            }
        }
    }

    fn build_sdp_context(&self) -> RtcSdpContext {
        let runtime_config = self
            .runtime_config
            .lock()
            .ok()
            .map(|config| config.clone())
            .unwrap_or_default();
        let target_type = self.last_request.lock().ok().and_then(|request| {
            request
                .as_ref()
                .map(|value| value.session.target_type.clone())
        });
        RtcSdpContext {
            negotiation: runtime_config.webrtc.negotiation,
            session_target_type: target_type,
        }
    }

    fn pump_connection_and_media_ingress(&self) {
        crate::xbx_log_debug!("[xbxengine][rtc-stack] pump_connection_and_media_ingress enter");
        self.record_transport_fact(TransportFact::Timer(TimerFact::MetricsSampleTick {
            observed_at_ms: crate::transport::rtc::stats::now_ms_f64(),
        }));
        let (ingress_packets, connection_facts) = self
            .connection
            .lock()
            .ok()
            .map(|mut connection| {
                crate::xbx_log_debug!("[xbxengine][rtc-stack] pump_connection lock acquired");
                let _ = connection.pump(&self.runtime_stats);
                (
                    connection.take_media_ingress_packets(),
                    connection.take_transport_facts(),
                )
            })
            .unwrap_or_default();
        for fact in connection_facts {
            self.record_transport_fact(fact);
        }
        crate::xbx_log_debug!(
            "[xbxengine][rtc-stack] pump_connection_and_media_ingress ingress_packets={}",
            ingress_packets.len()
        );
        if !ingress_packets.is_empty() {
            if let Ok(mut media) = self.media.lock() {
                for (packet, rtp_meta) in ingress_packets {
                    // 第一阶段只把连接层原始包送进媒体入口，不进入组帧/送解链。
                    media.observe_ingress_packet(packet, rtp_meta, &self.runtime_stats);
                }
            }
        }
        self.drain_transport_fact_sink();
        crate::xbx_log_debug!("[xbxengine][rtc-stack] pump_connection_and_media_ingress exit");
    }

    fn record_transport_fact(&self, fact: TransportFact) {
        let mut pending_commands = Vec::new();
        if let Ok(mut session) = self.transport_session.lock() {
            session.enqueue_fact(fact);
            let _ = session.drain_once(64);
            while let Some(command) = session.pop_next_command() {
                pending_commands.push(command);
            }
        }
        for command in pending_commands {
            self.apply_transport_session_command(command);
        }
    }

    fn reset_transport_session(&self) {
        if let Ok(mut session) = self.transport_session.lock() {
            *session = SessionActor::new(SystemSessionClock, RtcSessionPolicy::default());
        }
    }

    fn drain_transport_fact_sink(&self) {
        let facts = self
            .transport_fact_sink
            .lock()
            .ok()
            .map(|mut pending| std::mem::take(&mut *pending))
            .unwrap_or_default();
        for fact in facts {
            self.record_transport_fact(fact);
        }
    }

    fn record_transport_command_result(
        &self,
        command: TransportCommand,
        result: &Result<(), XbxEngineRuntimeError>,
    ) {
        let status = match result {
            Ok(()) => CommandResultStatus::Succeeded,
            Err(error) => CommandResultStatus::Failed {
                error: error.to_string(),
            },
        };
        self.record_transport_fact(TransportFact::CommandResult(CommandResultFact {
            command,
            status,
            observed_at_ms: crate::transport::rtc::stats::now_ms_f64(),
        }));
    }

    fn apply_transport_session_command(&self, command: TransportCommand) {
        match &command {
            TransportCommand::RequestReconnectCandidate {
                observation_id,
                reason,
            } => {
                let result = self
                    .pending_runtime_recovery_action
                    .lock()
                    .map_err(|_| {
                        XbxEngineRuntimeError::new("xbxEngineRtcPendingRecoveryActionLockFailed")
                    })
                    .map(|mut pending| {
                        stage_reconnect_candidate(&mut pending, *observation_id, reason.clone());
                    });
                self.record_transport_command_result(command, &result.map(|_| ()));
            }
            TransportCommand::RequestKeyframe { .. } => {
                let result = self
                    .connection
                    .lock()
                    .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcConnectionLockFailed"))
                    .and_then(|mut connection| {
                        connection.request_video_keyframe(&self.runtime_stats)
                    });
                self.record_transport_command_result(command, &result);
            }
            TransportCommand::RequestDecoderReset { .. } => {
                let result = self
                    .connection
                    .lock()
                    .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcConnectionLockFailed"))
                    .and_then(|mut connection| {
                        connection.request_decoder_reset(&self.runtime_stats)
                    });
                self.record_transport_command_result(command, &result);
            }
            TransportCommand::SetTargetRembKbps {
                target_kbps,
                reason,
                ..
            } => {
                RuntimeStatsSink::new(self.runtime_stats.clone()).update(|stats| {
                    stats.video_remb_bps = Some(target_kbps.saturating_mul(1_000));
                    stats.latest_observation_label =
                        Some("rtcSessionCommandUpdateTargetRemb".to_string());
                    stats.latest_observation_summary = Some(format!(
                        "rtc session command updated target remb={}kbps reason={reason}",
                        target_kbps
                    ));
                });
                self.record_transport_command_result(command, &Ok(()));
            }
        }
    }

    fn collect_gamepad_frames(input_state: &mut RtcInputStreamState) -> Vec<LogicalPadSnapshotDto> {
        let Ok(host) = GamepadRuntimeHost::shared() else {
            return Vec::new();
        };
        let Ok(snapshot) = host.snapshot() else {
            return Vec::new();
        };
        if !matches!(
            snapshot.route_target,
            OhMyGamepadRouteTargetDto::StreamSession { .. }
        ) {
            return Vec::new();
        }
        let mut frames = Vec::with_capacity(4);
        let mut sample_count = 0usize;
        for frame in snapshot.pads.iter().take(4) {
            if sample_count < input_state.gamepad_sample_signature.len() {
                input_state.gamepad_sample_signature[sample_count] = frame.sample_seq;
            }
            sample_count += 1;
            frames.push(frame.clone());
        }
        input_state.gamepad_sample_signature[sample_count..].fill(0);
        frames
    }

    fn pump_input_stream_once(
        connection: &Arc<Mutex<RtcConnectionService>>,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        data_channel_state: &Arc<Mutex<XbxDataChannelState>>,
        input_stream_state: &Arc<Mutex<RtcInputStreamState>>,
    ) {
        let Ok(mut input_state) = input_stream_state.lock() else {
            return;
        };
        let (metadata, pointer_events, mouse_frames, keyboard_frames, frames, now_ms) = {
            let metadata = runtime_stats.lock().ok().and_then(|stats| {
                build_metadata_frame(&stats, &mut input_state.last_metadata_frame_seq)
            });
            let frames = Self::collect_gamepad_frames(&mut input_state);
            let sample_count = frames.len();
            let (pointer_events, mouse_frames, keyboard_frames) =
                drain_pending_input_frames(data_channel_state, runtime_stats);
            let gamepad_changed = sample_count > 0
                && !(sample_count == input_state.last_gamepad_sample_count
                    && input_state.gamepad_sample_signature[..sample_count]
                        == input_state.last_gamepad_sample_signature
                            [..input_state.last_gamepad_sample_count]);
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            let should_send_idle_gamepad_keepalive = sample_count > 0
                && !gamepad_changed
                && pointer_events.is_empty()
                && mouse_frames.is_empty()
                && keyboard_frames.is_empty()
                && (now_ms - input_state.last_input_packet_sent_at_ms)
                    >= STREAM_INPUT_IDLE_GAMEPAD_KEEPALIVE_MS as f64;
            if metadata.is_none()
                && !gamepad_changed
                && !should_send_idle_gamepad_keepalive
                && pointer_events.is_empty()
                && mouse_frames.is_empty()
                && keyboard_frames.is_empty()
            {
                return;
            }
            let gamepad_frames = if gamepad_changed || should_send_idle_gamepad_keepalive {
                frames
            } else {
                Vec::new()
            };
            (
                metadata,
                pointer_events,
                mouse_frames,
                keyboard_frames,
                gamepad_frames,
                now_ms,
            )
        };

        let packet = build_input_stream_packet(
            input_state.input_sequence,
            now_ms,
            metadata.as_ref(),
            &frames,
            &pointer_events,
            &mouse_frames,
            &keyboard_frames,
        );

        let sent = connection
            .lock()
            .ok()
            .and_then(|mut connection| {
                connection
                    .send_input_stream_packet(packet, runtime_stats)
                    .ok()
            })
            .unwrap_or(false);
        if !sent {
            return;
        }
        input_state.input_sequence = input_state.input_sequence.wrapping_add(1);
        input_state.last_input_packet_sent_at_ms = now_ms;
        if !frames.is_empty() {
            input_state.last_gamepad_sample_count = frames.len();
            let current_signature = input_state.gamepad_sample_signature;
            input_state.last_gamepad_sample_signature[..frames.len()]
                .copy_from_slice(&current_signature[..frames.len()]);
            input_state.last_gamepad_sample_signature[frames.len()..].fill(0);
        }
    }
}

impl XbxMediaStackPort for XbxActiveMediaStack {
    fn sync_runtime_config(&mut self, runtime_config: XbxEngineRuntimeConfig) {
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

    fn rebuild_peer_connection(
        &mut self,
        request: &XbxEngineMediaNegotiationRequest,
    ) -> Result<(), XbxEngineRuntimeError> {
        let _ = &self.media_runtime;
        self.reset_transport_session();
        self.record_transport_fact(TransportFact::Peer(PeerFact::ConnectionStateChanged {
            state: ConnectionLifecycleStateFact::Connecting,
            observed_at_ms: crate::transport::rtc::stats::now_ms_f64(),
        }));
        if let Ok(mut render_state) = self.render_state.lock() {
            render_state.reset()?;
        }
        if let Ok(mut pending_action) = self.pending_runtime_recovery_action.lock() {
            *pending_action = None;
        }
        if let Ok(mut data_channel_state) = self.data_channel_state.lock() {
            *data_channel_state = XbxDataChannelState::default();
        }
        if let Ok(mut stats) = self.runtime_stats.lock() {
            *stats = XbxEngineMediaRuntimeStats {
                session_target_type: Some(request.session.target_type.clone()),
                ..Default::default()
            };
        }
        if let Ok(mut last_request) = self.last_request.lock() {
            *last_request = Some(request.clone());
        }
        if let Ok(mut media) = self.media.lock() {
            media.reset();
        }
        if let Ok(mut input_state) = self.input_stream_state.lock() {
            *input_state = RtcInputStreamState::default();
        }
        self.ensure_input_loop_running();
        self.mount_legacy_frame_pipeline();
        if let Ok(mut connection) = self.connection.lock() {
            if let Ok(config) = self.runtime_config.lock() {
                connection.sync_runtime_config(config.webrtc.clone());
            }
        }
        self.connection
            .lock()
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcConnectionLockFailed"))?
            .rebuild(&request.session, &self.runtime_stats)
    }

    fn create_offer(&self) -> Result<String, XbxEngineRuntimeError> {
        let runtime_config = self
            .runtime_config
            .lock()
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcRuntimeConfigLockFailed"))?
            .clone();
        let raw_offer = self
            .connection
            .lock()
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcConnectionLockFailed"))?
            .create_raw_offer(&runtime_config.webrtc.negotiation, &self.runtime_stats)?;
        let adapted_offer = adapt_local_offer(&raw_offer, &self.build_sdp_context());
        validate_local_offer_sdp(&adapted_offer)?;
        crate::xbx_log_info!(
            "[xbxengine][rtc-phase1] local offer created {}",
            summarize_sdp(&adapted_offer)
        );
        Ok(adapted_offer)
    }

    fn apply_remote_description(
        &self,
        answer_sdp: &str,
        remote_candidates: &[XbxEngineIceCandidateDto],
    ) -> Result<(), XbxEngineRuntimeError> {
        let normalized_answer = adapt_remote_answer(answer_sdp);
        let normalized_candidates = remote_candidates
            .iter()
            .filter_map(normalize_remote_candidate)
            .collect::<Vec<_>>();
        if let Ok(mut media) = self.media.lock() {
            media.apply_remote_answer_sdp(&normalized_answer);
        }
        self.connection
            .lock()
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcConnectionLockFailed"))?
            .apply_remote_description(
                &normalized_answer,
                &normalized_candidates,
                &self.runtime_stats,
            )
    }

    fn add_remote_ice_candidates(
        &self,
        remote_candidates: &[XbxEngineIceCandidateDto],
    ) -> Result<(), XbxEngineRuntimeError> {
        let normalized_candidates = remote_candidates
            .iter()
            .filter_map(normalize_remote_candidate)
            .collect::<Vec<_>>();
        self.connection
            .lock()
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcConnectionLockFailed"))?
            .add_remote_ice_candidates(&normalized_candidates, &self.runtime_stats)
    }

    fn apply_display_state(
        &mut self,
        state: XbxEngineDisplayStateDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        let mut render_state = self
            .render_state
            .lock()
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRenderStateLockFailed"))?;
        render_state.apply_display_state(state)
    }

    fn local_candidates_snapshot(&self) -> Vec<XbxEngineIceCandidateDto> {
        self.pump_connection_and_media_ingress();
        let candidates = self
            .connection
            .lock()
            .ok()
            .map(|connection| connection.local_candidates_snapshot())
            .unwrap_or_default();
        crate::xbx_log_info!(
            "[xbxengine][rtc-stack] local_candidates_snapshot count={}",
            candidates.len()
        );
        candidates
    }

    fn local_ice_gathering_complete(&self) -> bool {
        self.pump_connection_and_media_ingress();
        let complete = self
            .connection
            .lock()
            .ok()
            .is_some_and(|connection| connection.local_ice_gathering_complete());
        crate::xbx_log_info!(
            "[xbxengine][rtc-stack] local_ice_gathering_complete complete={complete}"
        );
        complete
    }

    fn snapshot_runtime_stats(&self) -> XbxEngineMediaRuntimeStats {
        crate::xbx_log_debug!("[xbxengine][rtc-stack] snapshot_runtime_stats enter");
        self.pump_connection_and_media_ingress();
        crate::xbx_log_debug!("[xbxengine][rtc-stack] snapshot_runtime_stats after pump");
        let mut stats = self
            .runtime_stats
            .lock()
            .ok()
            .map(|guard| guard.clone())
            .unwrap_or_default();
        crate::xbx_log_debug!("[xbxengine][rtc-stack] snapshot_runtime_stats runtime_stats cloned");
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        if let Ok(render_state) = self.render_state.lock() {
            let render_signal = render_state.render_signal_snapshot(now_ms);
            stats.latest_video_present_time_ms = render_signal.latest_present_time_ms;
            stats.video_present_fps = render_signal.fps;
            stats.video_renderer_stalled = render_signal.renderer_stalled;
        }
        if let Ok(media) = self.media.lock() {
            let media_snapshot = media.snapshot();
            stats.inbound_audio_bytes_total = media_snapshot.inbound_audio_bytes;
            stats.inbound_bytes_total =
                stats.inbound_video_bytes_total + stats.inbound_audio_bytes_total;
            if stats.latest_video_track_status.is_none()
                && media_snapshot.inbound_audio_bytes > 0
                && stats.inbound_video_bytes_total == 0
            {
                // 音频-only 情况下，补一个可消费的 track 状态，避免统计面板一直空白。
                stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                    state: "audioOnly".to_string(),
                    video_width: None,
                    video_height: None,
                    mime_type: None,
                    transport_state: stats.transport_state.clone(),
                    video_bytes_total: 0,
                    video_packet_count_total: 0,
                    audio_bytes_total: stats.inbound_audio_bytes_total,
                    observed_at_ms: now_ms,
                });
            }
        }
        crate::xbx_log_debug!("[xbxengine][rtc-stack] snapshot_runtime_stats exit");
        stats
    }

    fn take_pending_runtime_recovery_action(
        &mut self,
    ) -> Option<XbxEnginePendingRuntimeRecoveryAction> {
        self.pending_runtime_recovery_action
            .lock()
            .ok()
            .and_then(|mut action| action.take())
    }

    fn take_latest_render_frame(&mut self) -> Option<XbxEngineRenderFrame> {
        self.render_state
            .lock()
            .ok()
            .and_then(|mut render_state| render_state.take_latest_frame())
    }

    fn set_audio_volume(&mut self, _value: f32) {}

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

    fn request_video_keyframe(&mut self) -> Result<(), XbxEngineRuntimeError> {
        let result = self
            .connection
            .lock()
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcConnectionLockFailed"))?
            .request_video_keyframe(&self.runtime_stats);
        self.record_transport_command_result(
            TransportCommand::RequestKeyframe {
                reason: "stack.manualRequest".to_string(),
                observation_id: 0,
            },
            &result,
        );
        result
    }

    fn request_decoder_reset(&mut self) -> Result<(), XbxEngineRuntimeError> {
        let result = self
            .connection
            .lock()
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcConnectionLockFailed"))?
            .request_decoder_reset(&self.runtime_stats);
        self.record_transport_command_result(
            TransportCommand::RequestDecoderReset {
                reason: "stack.manualRequest".to_string(),
                observation_id: 0,
            },
            &result,
        );
        result
    }

    fn update_host_video_timing(
        &mut self,
        host_display_interval_ms: Option<f64>,
        host_frame_age_budget_ms: Option<f64>,
    ) {
        RuntimeStatsSink::new(self.runtime_stats.clone())
            .record_host_video_timing(host_display_interval_ms, host_frame_age_budget_ms);
    }

    fn stop(&mut self) {
        self.input_loop_stop.store(true, Ordering::Relaxed);
        if let Some(task) = self.input_loop_task.take() {
            task.abort();
        }
        if let Ok(mut connection) = self.connection.lock() {
            connection.stop(&self.runtime_stats);
        }
        if let Ok(mut pending_action) = self.pending_runtime_recovery_action.lock() {
            *pending_action = None;
        }
        if let Ok(mut render_state) = self.render_state.lock() {
            render_state.stop();
        }
        self.record_transport_fact(TransportFact::Peer(PeerFact::ConnectionStateChanged {
            state: ConnectionLifecycleStateFact::Closed,
            observed_at_ms: crate::transport::rtc::stats::now_ms_f64(),
        }));
    }
}
