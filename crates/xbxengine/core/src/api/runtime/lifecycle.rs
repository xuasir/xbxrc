use xbxengine_protocol::{
    XbxEngineControlCommandDto, XbxEngineDisplayStateDto, XbxEngineHostRequestDto,
    XbxEngineHostResponseDto, XbxEngineIceCandidateDto, XbxEngineReconnectReasonDto,
    XbxEngineRenderProjectionDto, XbxEngineRuntimeEventDto, XbxEngineRuntimePhaseDto,
    XbxEngineRuntimeProjectionDto, XbxEngineTransportStateDto, XbxEngineViewportDto,
};

use super::{
    dedupe_remote_ice_candidates, ice_candidate_dedupe_key, is_end_of_candidates_marker,
    now_ms_f64, XbxEngineEventSink, XbxEngineHostBridge, XbxEngineRuntime, XbxEngineRuntimeError,
    XbxEngineRuntimeState, XbxEngineVideoPipelineRuntimeConfig,
};
use crate::{
    XbxEngineMediaBackend, XbxEngineMediaNegotiationRequest, XbxEngineRecoveryRuntimeConfig,
};

impl<THostBridge, TEventSink, TMediaBackend>
    XbxEngineRuntime<THostBridge, TEventSink, TMediaBackend>
where
    THostBridge: XbxEngineHostBridge,
    TEventSink: XbxEngineEventSink,
    TMediaBackend: XbxEngineMediaBackend,
{
    pub fn start(
        &mut self,
        session: xbxengine_protocol::XbxEngineSessionDto,
        viewport: XbxEngineViewportDto,
        audio_volume: f32,
        runtime: Option<XbxEngineRuntimeProjectionDto>,
        render: Option<XbxEngineRenderProjectionDto>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let previous_config = self.config.clone();
        let previous_state = self.state.clone();
        let previous_session = self.session.clone();
        let previous_snapshot = self.snapshot.clone();
        let previous_health = self.health.clone();
        self.state = XbxEngineRuntimeState::Starting;
        self.session = Some(session);
        self.snapshot.viewport = Some(viewport);
        self.snapshot.audio_volume = audio_volume;
        let operation_epoch = self.host_bridge.current_cancellation_epoch();

        let start_result = (|| {
            self.apply_execution_spec(runtime.as_ref(), render.as_ref())?;
            self.media_backend.sync_runtime_config(&self.config)?;
            self.media_backend.set_audio_volume(audio_volume)?;
            self.emit_phase(XbxEngineRuntimePhaseDto::Binding);
            self.ensure_operation_active(operation_epoch)?;
            self.negotiate_remote(false, operation_epoch)?;
            Ok(())
        })();

        match start_result {
            Ok(()) => {
                self.state = XbxEngineRuntimeState::Running;
                Ok(())
            }
            Err(error) => {
                let _ = self.media_backend.stop();
                self.config = previous_config;
                self.state = previous_state;
                self.session = previous_session;
                self.snapshot = previous_snapshot;
                self.health = previous_health;
                Err(error)
            }
        }
    }

    pub fn request_reconnect(
        &mut self,
        reason: XbxEngineReconnectReasonDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        let previous_state = self.state.clone();
        let previous_session = self.session.clone();
        let previous_snapshot = self.snapshot.clone();
        let previous_health = self.health.clone();
        let session_id = self.require_session_id()?;
        let reconnect_started_at_ms = now_ms_f64();
        let operation_epoch = self.host_bridge.current_cancellation_epoch();
        self.state = XbxEngineRuntimeState::Reconnecting;
        self.health.mark_reconnect_started(reconnect_started_at_ms);
        self.snapshot.recovery_reconnect_count =
            self.snapshot.recovery_reconnect_count.saturating_add(1);
        self.snapshot.last_recovery_action = Some("reconnect".to_string());
        self.snapshot.last_recovery_action_at_ms = Some(reconnect_started_at_ms);
        self.snapshot.last_recovery_reason = Some(format!("{reason:?}"));
        self.emit_phase(XbxEngineRuntimePhaseDto::Reconnecting);

        let reconnect_result = (|| {
            self.ensure_operation_active(operation_epoch)?;
            let _ = self
                .host_bridge
                .request(XbxEngineHostRequestDto::KeepAliveRemoteSession { session_id })?;
            self.ensure_operation_active(operation_epoch)?;
            self.negotiate_remote(true, operation_epoch)?;
            if self.snapshot.microphone_capturing {
                self.ensure_operation_active(operation_epoch)?;
                self.media_backend.set_microphone_capturing(true)?;
                self.renegotiate_chat_channel()?;
            }
            Ok(())
        })();

        match reconnect_result {
            Ok(()) => {
                self.state = XbxEngineRuntimeState::Running;
                Ok(())
            }
            Err(error) => {
                self.state = previous_state;
                self.session = previous_session;
                self.snapshot = previous_snapshot;
                self.health = previous_health;
                self.health
                    .restore_reconnect_marker(reconnect_started_at_ms);
                Err(error)
            }
        }
    }

    pub fn stop(&mut self) {
        let viewport_id = self
            .snapshot
            .viewport
            .as_ref()
            .map(|viewport| viewport.viewport_id.as_str());
        if let Err(error) = self.host_bridge.detach_viewport(viewport_id) {
            self.emit_error("detachViewportFailed", error.to_string());
        }
        self.session = None;
        if let Err(error) = self.media_backend.stop() {
            self.emit_error("stopMediaBackendFailed", error.to_string());
        }
        self.snapshot.viewport = None;
        self.snapshot.surface_id = None;
        self.snapshot.video_size = None;
        self.health = crate::XbxEngineRuntimeHealth::default();
        self.state = XbxEngineRuntimeState::Stopped;
        self.emit_transport_state(XbxEngineTransportStateDto::Closed);
    }

    pub fn tick(&mut self) {
        if !matches!(
            self.state,
            XbxEngineRuntimeState::Running | XbxEngineRuntimeState::Reconnecting
        ) {
            return;
        }

        let runtime_stats = match self.media_backend.snapshot_runtime_stats() {
            Ok(stats) => stats,
            Err(error) => {
                self.emit_error("snapshotMediaRuntimeStatsFailed", error.to_string());
                return;
            }
        };

        self.present_latest_render_frame();
        self.sync_transport_state(&runtime_stats);
        self.sync_video_packet_stats(&runtime_stats);
        self.sync_video_frame_stats(&runtime_stats);
        if self.maybe_consume_pending_runtime_recovery_action() {
            return;
        }
    }

    fn maybe_consume_pending_runtime_recovery_action(&mut self) -> bool {
        let Ok(action) = self.media_backend.take_pending_runtime_recovery_action() else {
            return false;
        };
        let Some(action) = action else {
            return false;
        };
        let reason = match action {
            crate::XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
                reason,
                ..
            } => reason,
        };
        if let Err(error) = self.request_reconnect(XbxEngineReconnectReasonDto::MediaStalled) {
            if !error.is_cancelled() {
                if is_terminal_remote_session_inactive_error(&error) {
                    self.snapshot.last_recovery_action =
                        Some("reconnectSessionInactive".to_string());
                    self.snapshot.last_recovery_action_at_ms = Some(now_ms_f64());
                    self.snapshot.last_recovery_reason =
                        Some("transportReconnectCandidate:sessionNotActive".to_string());
                    self.emit_error(
                        "recoverTransportReconnectSessionNotActive",
                        error.to_string(),
                    );
                    self.stop();
                    return true;
                }
                self.emit_error(
                    "recoverTransportReconnectCandidateFailed",
                    error.to_string(),
                );
            }
        } else {
            self.snapshot.last_recovery_reason =
                Some(format!("transportReconnectCandidate:{reason}"));
        }
        true
    }

    pub fn apply_control(
        &mut self,
        command: XbxEngineControlCommandDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        match command {
            XbxEngineControlCommandDto::StartRuntime {
                session,
                viewport,
                audio_volume,
                runtime,
                render,
            } => self.start(session, viewport, audio_volume, runtime, render),
            XbxEngineControlCommandDto::StopRuntime => {
                self.stop();
                Ok(())
            }
            XbxEngineControlCommandDto::RequestReconnect { reason } => {
                self.request_reconnect(reason)
            }
            XbxEngineControlCommandDto::AttachViewport { viewport } => {
                self.host_bridge
                    .attach_viewport(&viewport, self.snapshot.surface_id.as_deref())?;
                self.snapshot.viewport = Some(viewport);
                Ok(())
            }
            XbxEngineControlCommandDto::DetachViewport => {
                let viewport_id = self
                    .snapshot
                    .viewport
                    .as_ref()
                    .map(|viewport| viewport.viewport_id.as_str());
                self.host_bridge.detach_viewport(viewport_id)?;
                self.snapshot.viewport = None;
                Ok(())
            }
            XbxEngineControlCommandDto::ApplyDisplayState { state } => {
                self.media_backend.apply_display_state(state.clone())?;
                self.snapshot.display_state = Some(state);
                Ok(())
            }
            XbxEngineControlCommandDto::SetAudioVolume { value } => {
                self.media_backend.set_audio_volume(value)?;
                self.snapshot.audio_volume = value;
                Ok(())
            }
            XbxEngineControlCommandDto::StartMicrophone => {
                self.media_backend.set_microphone_capturing(true)?;
                if let Err(error) = self.renegotiate_chat_channel() {
                    let _ = self.media_backend.set_microphone_capturing(false);
                    return Err(error);
                }
                self.snapshot.microphone_capturing = true;
                self.snapshot.microphone_paused = false;
                self.event_sink
                    .emit(XbxEngineRuntimeEventDto::ChatStateChanged {
                        capturing: true,
                        paused: false,
                    });
                Ok(())
            }
            XbxEngineControlCommandDto::StopMicrophone => {
                self.media_backend.set_microphone_capturing(false)?;
                if let Err(error) = self.renegotiate_chat_channel() {
                    let _ = self.media_backend.set_microphone_capturing(true);
                    return Err(error);
                }
                self.snapshot.microphone_capturing = false;
                self.snapshot.microphone_paused = true;
                self.event_sink
                    .emit(XbxEngineRuntimeEventDto::ChatStateChanged {
                        capturing: false,
                        paused: true,
                    });
                Ok(())
            }
            XbxEngineControlCommandDto::PressControllerButton {
                button,
                duration_ms,
            } => {
                self.media_backend
                    .press_controller_button(button.clone(), duration_ms)?;
                let input_status = self.media_backend.current_input_status()?;
                self.record_input_status(&input_status);
                self.snapshot.last_pressed_controller_button = Some((button, duration_ms));
                Ok(())
            }
            XbxEngineControlCommandDto::SetKeyboardPointerEnabled { enabled } => {
                self.media_backend.set_keyboard_pointer_enabled(enabled)?;
                self.snapshot.keyboard_pointer_enabled = enabled;
                Ok(())
            }
            XbxEngineControlCommandDto::PushKeyboardPointerInput { event } => {
                self.media_backend
                    .push_keyboard_pointer_input(event.clone())?;
                self.snapshot.last_keyboard_pointer_event = Some(event);
                Ok(())
            }
        }
    }

    fn apply_execution_spec(
        &mut self,
        runtime: Option<&XbxEngineRuntimeProjectionDto>,
        render: Option<&XbxEngineRenderProjectionDto>,
    ) -> Result<(), XbxEngineRuntimeError> {
        if let Some(runtime) = runtime {
            if let Some(video_bitrate_kbps) = runtime.max_video_bitrate_kbps {
                self.config.webrtc.negotiation.video_bitrate_kbps = video_bitrate_kbps;
            }
            if let Some(audio_bitrate_kbps) = runtime.max_audio_bitrate_kbps {
                self.config.webrtc.negotiation.audio_bitrate_kbps = audio_bitrate_kbps;
            }
            self.config.webrtc.negotiation.force_mono_audio = runtime.force_mono_audio;
            self.config.webrtc.negotiation.target_resolution_width = runtime.target_video_width;
            self.config.webrtc.negotiation.target_resolution_height = runtime.target_video_height;
            self.config.webrtc.forced_remb_kbps = runtime.forced_remb_kbps;
            self.config.webrtc.adaptive_remb_enabled = runtime.adaptive_remb_enabled;
            self.config.webrtc.bwe_mode = runtime.bwe_mode.clone();
            self.config.webrtc.remb_floor_kbps = runtime.remb_floor_kbps;
            self.config.webrtc.remb_ceiling_kbps = runtime.remb_ceiling_kbps;
            self.config.webrtc.remb_ramp_up_step_kbps = runtime.remb_ramp_up_step_kbps;
            self.config.webrtc.remb_ramp_down_factor = runtime.remb_ramp_down_factor;
            self.config.webrtc.video_pipeline = XbxEngineVideoPipelineRuntimeConfig {
                feedback_interval_ms: runtime.video_pipeline.feedback_interval_ms,
                nack_window_ms: runtime.video_pipeline.nack_window_ms,
                nack_burst_count: runtime.video_pipeline.nack_burst_count,
                nack_max_age_ms: runtime.video_pipeline.nack_max_age_ms,
                nack_retry_interval_ms: runtime.video_pipeline.nack_retry_interval_ms,
                nack_max_retry_count: runtime.video_pipeline.nack_max_retry_count,
                jitter_buffer_min_delay_ms: runtime.video_pipeline.jitter_buffer_min_delay_ms,
                jitter_buffer_max_delay_ms: runtime.video_pipeline.jitter_buffer_max_delay_ms,
                jitter_buffer_max_packets: runtime.video_pipeline.jitter_buffer_max_packets,
                idle_timeout_ms: runtime.video_pipeline.idle_timeout_ms,
                late_frame_drop_threshold_ms: runtime.video_pipeline.late_frame_drop_threshold_ms,
                backlog_drop_threshold_packets: runtime
                    .video_pipeline
                    .backlog_drop_threshold_packets,
            };
            self.config.webrtc.recovery = XbxEngineRecoveryRuntimeConfig {
                first_frame_grace_ms: runtime.recovery.first_frame_grace_ms,
                keyframe_request_stall_ms: runtime.recovery.keyframe_request_stall_ms,
                keyframe_loss_burst_threshold: runtime.recovery.keyframe_loss_burst_threshold,
                decoder_reset_after_keyframe_wait_ms: runtime
                    .recovery
                    .decoder_reset_after_keyframe_wait_ms,
                decoder_reset_request_cooldown_ms: runtime
                    .recovery
                    .decoder_reset_request_cooldown_ms,
                reconnect_stall_ms: runtime.recovery.reconnect_stall_ms,
                stall_recovery_cooldown_ms: runtime.recovery.stall_recovery_cooldown_ms,
            };
            if let Some(codec) = runtime.codec.as_ref() {
                if let Some(profile) = codec.profiles.first() {
                    self.config.webrtc.negotiation.offer_profile =
                        normalize_offer_profile_token(profile);
                }
            }
        }

        if let Some(render) = render {
            let display_state = XbxEngineDisplayStateDto {
                display_options: render.display_options.clone(),
            };
            self.media_backend
                .apply_display_state(display_state.clone())?;
            self.snapshot.display_state = Some(display_state);
        }

        Ok(())
    }

    fn renegotiate_chat_channel(&mut self) -> Result<(), XbxEngineRuntimeError> {
        let local_offer_sdp = self.media_backend.create_offer()?;
        let answer_sdp = Self::extract_offer_response(self.host_bridge.request(
            XbxEngineHostRequestDto::ExchangeOffer {
                session_id: self.require_session_id()?,
                channel: "chat".to_string(),
                sdp: local_offer_sdp.clone(),
                restart: false,
            },
        )?)?;
        self.media_backend
            .apply_remote_description(answer_sdp.clone(), Vec::new())?;
        self.snapshot.last_offer_sdp = Some(local_offer_sdp);
        self.snapshot.last_answer_sdp = Some(answer_sdp);
        Ok(())
    }

    fn negotiate_remote(
        &mut self,
        restart: bool,
        operation_epoch: u64,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.ensure_operation_active(operation_epoch)?;
        let negotiation = self
            .media_backend
            .negotiate(XbxEngineMediaNegotiationRequest {
                session: self.require_session()?.clone(),
                viewport: self.require_viewport()?.clone(),
                restart,
            })?;

        self.snapshot.negotiation_attempt_count += 1;
        self.snapshot.last_offer_sdp = Some(negotiation.local_offer_sdp.clone());

        self.emit_phase(XbxEngineRuntimePhaseDto::ExchangingOffer);
        self.ensure_operation_active(operation_epoch)?;
        let answer_sdp = Self::extract_offer_response(self.host_bridge.request(
            XbxEngineHostRequestDto::ExchangeOffer {
                session_id: self.require_session_id()?,
                channel: "media".to_string(),
                sdp: negotiation.local_offer_sdp.clone(),
                restart,
            },
        )?)?;

        self.emit_phase(XbxEngineRuntimePhaseDto::GatheringIce);
        self.ensure_operation_active(operation_epoch)?;
        self.media_backend
            .apply_remote_description(answer_sdp.clone(), Vec::new())?;
        self.emit_phase(XbxEngineRuntimePhaseDto::Connecting);
        self.health.observed_transport_state = XbxEngineTransportStateDto::Connecting;
        self.emit_transport_state(XbxEngineTransportStateDto::Connecting);
        self.sync_runtime_activity_snapshot();
        let remote_candidates = self.exchange_remote_ice_incrementally(
            negotiation.local_candidates.clone(),
            restart,
            operation_epoch,
        )?;
        self.snapshot.last_answer_sdp = Some(answer_sdp);
        self.snapshot.last_remote_candidates = remote_candidates;
        self.record_media_ready(&negotiation);
        self.record_input_status(&negotiation.input_status);
        self.sync_runtime_activity_snapshot();
        Ok(())
    }

    fn exchange_remote_ice_incrementally(
        &mut self,
        initial_local_candidates: Vec<XbxEngineIceCandidateDto>,
        restart: bool,
        operation_epoch: u64,
    ) -> Result<Vec<XbxEngineIceCandidateDto>, XbxEngineRuntimeError> {
        use std::collections::HashSet;
        use std::time::Duration;

        let mut sent_local_candidates = HashSet::<String>::new();
        let mut applied_remote_candidates = HashSet::<String>::new();
        let mut aggregated_remote_candidates = Vec::new();
        let mut final_poll_sent = false;
        let mut remote_end_of_candidates_seen = false;

        loop {
            self.ensure_operation_active(operation_epoch)?;
            let local_candidates = self.collect_unsent_local_candidates(
                &initial_local_candidates,
                &mut sent_local_candidates,
            )?;
            let local_gathering_complete = self.media_backend.local_ice_gathering_complete()?;

            if local_candidates.is_empty() && !local_gathering_complete {
                std::thread::sleep(Duration::from_millis(60));
                continue;
            }

            if local_candidates.is_empty() && local_gathering_complete && final_poll_sent {
                break;
            }

            let request_candidates = if local_candidates.is_empty() {
                final_poll_sent = true;
                Vec::new()
            } else {
                local_candidates
            };

            self.emit_phase(XbxEngineRuntimePhaseDto::ExchangingIce);
            self.ensure_operation_active(operation_epoch)?;
            Self::extract_submit_ice_response(self.host_bridge.request(
                XbxEngineHostRequestDto::SubmitIce {
                    session_id: self.require_session_id()?,
                    candidates: request_candidates,
                    restart,
                },
            )?)?;
            self.ensure_operation_active(operation_epoch)?;
            let remote_candidates = Self::extract_poll_ice_response(self.host_bridge.request(
                XbxEngineHostRequestDto::PollIce {
                    session_id: self.require_session_id()?,
                    restart,
                },
            )?)?;
            self.ensure_operation_active(operation_epoch)?;
            remote_end_of_candidates_seen |= remote_candidates
                .iter()
                .any(|candidate| is_end_of_candidates_marker(&candidate.candidate));

            let next_remote_candidates =
                dedupe_remote_ice_candidates(remote_candidates, &mut applied_remote_candidates);
            if !next_remote_candidates.is_empty() {
                self.media_backend
                    .add_remote_ice_candidates(next_remote_candidates.clone())?;
                aggregated_remote_candidates.extend(next_remote_candidates);
            }

            self.sync_runtime_activity_snapshot();

            if local_gathering_complete
                && (remote_end_of_candidates_seen || self.health.connected_at_ms.is_some())
            {
                break;
            }
        }

        Ok(aggregated_remote_candidates)
    }

    fn ensure_operation_active(&self, operation_epoch: u64) -> Result<(), XbxEngineRuntimeError> {
        if self.host_bridge.current_cancellation_epoch() != operation_epoch {
            return Err(XbxEngineRuntimeError::new("xbxEngineRuntimeCancelled"));
        }
        Ok(())
    }

    fn collect_unsent_local_candidates(
        &self,
        initial_local_candidates: &[XbxEngineIceCandidateDto],
        sent_local_candidates: &mut std::collections::HashSet<String>,
    ) -> Result<Vec<XbxEngineIceCandidateDto>, XbxEngineRuntimeError> {
        let mut pending = Vec::new();
        for candidate in initial_local_candidates
            .iter()
            .cloned()
            .chain(self.media_backend.local_candidates_snapshot()?.into_iter())
        {
            let key = ice_candidate_dedupe_key(&candidate);
            if sent_local_candidates.insert(key) {
                pending.push(candidate);
            }
        }
        Ok(pending)
    }

    fn extract_offer_response(
        response: XbxEngineHostResponseDto,
    ) -> Result<String, XbxEngineRuntimeError> {
        match response {
            XbxEngineHostResponseDto::OfferExchanged { answer_sdp } => Ok(answer_sdp),
            _ => Err(XbxEngineRuntimeError::new(
                "xbxEngineHostBridgeInvalidOfferResponse",
            )),
        }
    }

    fn extract_submit_ice_response(
        response: XbxEngineHostResponseDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        match response {
            XbxEngineHostResponseDto::IceSubmitted => Ok(()),
            _ => Err(XbxEngineRuntimeError::new(
                "xbxEngineHostBridgeInvalidSubmitIceResponse",
            )),
        }
    }

    fn extract_poll_ice_response(
        response: XbxEngineHostResponseDto,
    ) -> Result<Vec<XbxEngineIceCandidateDto>, XbxEngineRuntimeError> {
        match response {
            XbxEngineHostResponseDto::IcePolled { candidates } => Ok(candidates),
            _ => Err(XbxEngineRuntimeError::new(
                "xbxEngineHostBridgeInvalidPollIceResponse",
            )),
        }
    }
}

fn normalize_offer_profile_token(profile: &str) -> String {
    let normalized = profile.trim().to_ascii_lowercase();
    normalized
        .strip_prefix("profile-level-id=")
        .unwrap_or(normalized.as_str())
        .to_string()
}

fn is_terminal_remote_session_inactive_error(error: &XbxEngineRuntimeError) -> bool {
    let message = error.to_string();
    let normalized = message.to_ascii_lowercase();
    let from_keepalive = normalized.contains("keepaliveremotesession")
        || normalized.contains("keepalive remote session");
    let session_inactive =
        normalized.contains("sessionnotactive") || normalized.contains("session not active");
    let http_410 = normalized.contains("http 410")
        || normalized.contains("status 410")
        || normalized.contains("code 410");
    from_keepalive && (session_inactive || http_410)
}
