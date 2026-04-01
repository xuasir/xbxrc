use ohmygamepad_protocol::OhMyGamepadRumbleEffectDto;
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
use crate::session::recovery::STALL_SIGNAL_STABILITY_MS;
use crate::{
    XbxEngineDecodeRenderSignal, XbxEngineMediaBackend, XbxEngineMediaNegotiationRequest,
    XbxEngineMediaSignal, XbxEngineRecoveryAction, XbxEngineRecoveryRuntimeConfig,
    XbxEngineRecoverySignals, XbxEngineTransportSignal,
};

const ICE_EXCHANGE_TIMEOUT_MS_MIN: f64 = 10_000.0;
const ICE_EXCHANGE_TIMEOUT_MS_MAX: f64 = 12_000.0;
const ICE_EXCHANGE_STABLE_SETTLE_WINDOW_MS: f64 = 1_500.0;
const TRANSPORT_RECONNECT_CANDIDATE_MIN_INTERVAL_MS: f64 = 6_000.0;

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
        self.drive_pending_gamepad_rumble_requests();
        if self.maybe_handle_terminal_session_kick(&runtime_stats) {
            return;
        }
        if self.maybe_consume_pending_runtime_recovery_action(&runtime_stats) {
            return;
        }
        // rust-owned 模式下，恢复动作统一由 transport session policy 主链裁决并执行；
        // runtime lifecycle 不再并行发 keyframe/reset/reconnect，避免双轨决策。
        if self.recovery_actions_owned_by_transport_policy() {
            return;
        }
        if self.drive_runtime_recovery_action(&runtime_stats) {
            return;
        }
    }

    fn recovery_actions_owned_by_transport_policy(&self) -> bool {
        self.config.runtime_name == "rust-owned"
    }

    fn maybe_consume_pending_runtime_recovery_action(
        &mut self,
        runtime_stats: &crate::XbxEngineMediaRuntimeStats,
    ) -> bool {
        if matches!(self.state, XbxEngineRuntimeState::Reconnecting) {
            self.snapshot.last_recovery_reason =
                Some("transportReconnectCandidateDeferred:reconnecting".to_string());
            return false;
        }
        if self.snapshot.last_recovery_action.as_deref() == Some("reconnect")
            && self
                .snapshot
                .last_recovery_action_at_ms
                .is_some_and(|last_at_ms| {
                    now_ms_f64() - last_at_ms < TRANSPORT_RECONNECT_CANDIDATE_MIN_INTERVAL_MS
                })
        {
            self.snapshot.last_recovery_reason =
                Some("transportReconnectCandidateDeferred:cooldown".to_string());
            return false;
        }
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
        // 只要 transport 已经产出 pending reconnect candidate，就应立即消费执行，
        // 不能再依赖外层连接态门控，否则会出现“已决策但不落地”。
        self.snapshot.last_recovery_action = Some("reconnectCandidateConsumed".to_string());
        self.snapshot.last_recovery_action_at_ms = Some(now_ms_f64());
        self.snapshot.last_recovery_reason = Some(format!(
            "transportReconnectCandidateConsumed:{reason}:transportRecovering={}",
            runtime_stats_indicate_transport_recovering(runtime_stats)
        ));
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

    fn maybe_handle_terminal_session_kick(
        &mut self,
        runtime_stats: &crate::XbxEngineMediaRuntimeStats,
    ) -> bool {
        if runtime_stats.latest_observation_label.as_deref()
            != Some("rtcSessionKickedForClosedGame")
        {
            return false;
        }
        self.snapshot.last_recovery_action = Some("sessionKickedStop".to_string());
        self.snapshot.last_recovery_action_at_ms = Some(now_ms_f64());
        self.snapshot.last_recovery_reason = Some("sessionKicked:KickForClosedGame".to_string());
        self.emit_error(
            "recoverTransportSessionKickedForClosedGame",
            runtime_stats
                .latest_observation_summary
                .clone()
                .unwrap_or_else(|| "kick reason=KickForClosedGame".to_string()),
        );
        self.stop();
        true
    }

    fn drive_runtime_recovery_action(
        &mut self,
        runtime_stats: &crate::XbxEngineMediaRuntimeStats,
    ) -> bool {
        if !matches!(self.state, XbxEngineRuntimeState::Running) {
            return false;
        }

        let now_ms = now_ms_f64();
        let decoder_stall_is_recent_and_aligned = runtime_stats.video_decoder_stalled == Some(true)
            && runtime_stats.video_renderer_stalled != Some(true)
            && runtime_stats
                .latest_video_present_time_ms
                .zip(runtime_stats.latest_video_decode_ok_time_ms)
                .is_some_and(|(present_at_ms, decode_ok_at_ms)| {
                    let stall_age_ms = now_ms - present_at_ms.min(decode_ok_at_ms);
                    let alignment_gap_ms = (present_at_ms - decode_ok_at_ms).abs();
                    stall_age_ms <= STALL_SIGNAL_STABILITY_MS && alignment_gap_ms <= 20.0
                });
        let signals = XbxEngineRecoverySignals {
            transport: XbxEngineTransportSignal {
                transport_connected: runtime_stats.transport_state
                    == XbxEngineTransportStateDto::Connected,
                connected_at_ms: self.health.connected_at_ms,
                latest_video_packet_arrival_at_ms: runtime_stats
                    .latest_video_packet_arrival_time_ms
                    .or(self.health.last_video_packet_arrival_at_ms),
                latest_twcc_feedback_at_ms: runtime_stats
                    .latest_video_twcc_observation
                    .as_ref()
                    .map(|observation| observation.observed_at_ms),
                latest_nack_sent_at_ms: runtime_stats
                    .latest_video_nack_observation
                    .as_ref()
                    .map(|observation| observation.observed_at_ms),
                latest_nack_recovered_at_ms: runtime_stats
                    .latest_video_nack_observation
                    .as_ref()
                    .filter(|observation| observation.action == "recovered")
                    .map(|observation| observation.observed_at_ms),
                latest_nack_expired_at_ms: runtime_stats
                    .latest_video_nack_observation
                    .as_ref()
                    .filter(|observation| observation.action == "expiredDeadline")
                    .map(|observation| observation.observed_at_ms),
                latest_nack_expired_frame_is_keyframe: runtime_stats
                    .latest_video_nack_observation
                    .as_ref()
                    .and_then(|observation| observation.frame_is_keyframe)
                    .unwrap_or(false),
                audio_stream_alive: runtime_stats.inbound_audio_bitrate_kbps.unwrap_or(0.0) > 0.1,
            },
            media: XbxEngineMediaSignal {
                // 显式 decoder stall 还很新、且 decode/present 时间基本同步时，
                // 允许恢复逻辑绕过“最近还有媒体活动”的抑制，优先打 keyframe。
                latest_frame_decoded_at_ms: if decoder_stall_is_recent_and_aligned {
                    None
                } else {
                    runtime_stats
                        .latest_video_decode_ok_time_ms
                        .or(self.snapshot.frame_decoded_time_ms)
                        .or(self.health.last_frame_rendered_at_ms)
                },
                latest_frame_rendered_at_ms: if decoder_stall_is_recent_and_aligned {
                    None
                } else {
                    runtime_stats
                        .latest_video_present_time_ms
                        .or(self.snapshot.frame_rendered_time_ms)
                        .or(self.health.last_frame_rendered_at_ms)
                },
            },
            decode_render: XbxEngineDecodeRenderSignal {
                decoder_stalled: runtime_stats.video_decoder_stalled,
                render_stalled: runtime_stats.video_renderer_stalled,
                allow_decoder_reset: true,
            },
        };

        let Some(action) = self.health.next_recovery_action_with_signals_and_config(
            now_ms,
            true,
            signals,
            &self.config.webrtc.recovery,
        ) else {
            return false;
        };

        match action {
            XbxEngineRecoveryAction::RequestVideoKeyframe => {
                if let Err(error) = self.media_backend.request_video_keyframe() {
                    if is_control_channel_not_ready_error(&error) {
                        return true;
                    }
                    self.emit_error("requestVideoKeyframeFailed", error.to_string());
                    return true;
                }
                self.health.mark_keyframe_requested(now_ms);
                self.snapshot.recovery_keyframe_request_count = self
                    .snapshot
                    .recovery_keyframe_request_count
                    .saturating_add(1);
                self.snapshot.last_recovery_action = Some("requestKeyframe".to_string());
                self.snapshot.last_recovery_action_at_ms = Some(now_ms);
                self.sync_runtime_activity_snapshot();
                true
            }
            XbxEngineRecoveryAction::RequestDecoderReset => {
                if let Err(error) = self.media_backend.request_decoder_reset() {
                    if is_control_channel_not_ready_error(&error) {
                        return true;
                    }
                    self.emit_error("requestDecoderResetFailed", error.to_string());
                    return true;
                }
                self.health.mark_decoder_reset_requested(now_ms);
                self.snapshot.recovery_decoder_reset_count =
                    self.snapshot.recovery_decoder_reset_count.saturating_add(1);
                self.snapshot.last_recovery_action = Some("requestDecoderReset".to_string());
                self.snapshot.last_recovery_action_at_ms = Some(now_ms);
                self.sync_runtime_activity_snapshot();
                true
            }
            XbxEngineRecoveryAction::RequestReconnect(reason) => {
                if let Err(error) = self.request_reconnect(reason) {
                    if !error.is_cancelled() {
                        self.emit_error("requestReconnectFailed", error.to_string());
                    }
                } else {
                    self.sync_runtime_activity_snapshot();
                }
                true
            }
        }
    }

    fn drive_pending_gamepad_rumble_requests(&mut self) {
        let Ok(rumble_requests) = self.media_backend.take_pending_gamepad_rumble_requests() else {
            self.emit_error(
                "takePendingGamepadRumbleRequestsFailed",
                "xbxEngineRuntimeMediaBackendRumbleQueueUnavailable",
            );
            return;
        };

        for request in rumble_requests {
            let result = if is_stop_gamepad_rumble_request(&request.effect) {
                self.host_bridge.stop_gamepad_rumble(request.target.clone())
            } else {
                self.host_bridge.play_gamepad_rumble(request.clone())
            };

            if let Err(error) = result {
                self.emit_error("dispatchGamepadRumbleFailed", error.to_string());
            }
        }
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
            XbxEngineControlCommandDto::StopRuntime { .. } => {
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
            self.config.webrtc.negotiation.prefer_ipv6 = runtime.prefer_ipv6;
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
        crate::xbx_log_warn!(
            "[xbxengine][runtime][ice] negotiate_remote start restart={restart} state={:?}",
            self.state
        );
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
        crate::xbx_log_warn!(
            "[xbxengine][runtime][ice] negotiate_remote prepared offer len={} local_candidates={} surface_id={} video={}x{}",
            negotiation.local_offer_sdp.len(),
            negotiation.local_candidates.len(),
            negotiation.surface_id,
            negotiation.video_width,
            negotiation.video_height,
        );

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
        crate::xbx_log_warn!(
            "[xbxengine][runtime][ice] exchange offer response received answer_len={}",
            answer_sdp.len()
        );

        self.emit_phase(XbxEngineRuntimePhaseDto::GatheringIce);
        self.ensure_operation_active(operation_epoch)?;
        self.media_backend
            .apply_remote_description(answer_sdp.clone(), Vec::new())?;
        crate::xbx_log_warn!(
            "[xbxengine][runtime][ice] remote description applied answer_len={} local_candidates_snapshot_pending={}",
            answer_sdp.len(),
            negotiation.local_candidates.len(),
        );
        self.emit_phase(XbxEngineRuntimePhaseDto::Connecting);
        self.health.observed_transport_state = XbxEngineTransportStateDto::Connecting;
        self.emit_transport_state(XbxEngineTransportStateDto::Connecting);
        crate::xbx_log_warn!("[xbxengine][runtime][ice] before sync_runtime_activity_snapshot");
        self.sync_runtime_activity_snapshot();
        crate::xbx_log_warn!(
            "[xbxengine][runtime][ice] after sync_runtime_activity_snapshot entering exchange loop"
        );
        let remote_candidates = self.exchange_remote_ice_incrementally(
            &negotiation.local_offer_sdp,
            negotiation.local_candidates.clone(),
            restart,
            operation_epoch,
        )?;
        crate::xbx_log_warn!(
            "[xbxengine][runtime][ice] exchange_remote_ice_incrementally returned remote_candidates={}",
            remote_candidates.len()
        );
        self.snapshot.last_answer_sdp = Some(answer_sdp);
        self.snapshot.last_remote_candidates = remote_candidates;
        self.record_media_ready(&negotiation);
        self.record_input_status(&negotiation.input_status);
        self.sync_runtime_activity_snapshot();
        Ok(())
    }

    fn exchange_remote_ice_incrementally(
        &mut self,
        local_offer_sdp: &str,
        initial_local_candidates: Vec<XbxEngineIceCandidateDto>,
        restart: bool,
        operation_epoch: u64,
    ) -> Result<Vec<XbxEngineIceCandidateDto>, XbxEngineRuntimeError> {
        use std::collections::HashSet;
        use std::time::Duration;

        let exchange_started_at_ms = now_ms_f64();
        let exchange_timeout_ms = resolve_ice_exchange_timeout_ms(
            self.config.webrtc.recovery.first_frame_grace_ms,
            self.config.webrtc.recovery.reconnect_stall_ms,
        );
        let offer_sdp_candidates = collect_local_offer_ice_candidates(local_offer_sdp);
        let mut sent_local_candidates = HashSet::<String>::new();
        let mut applied_remote_candidates = HashSet::<String>::new();
        let mut aggregated_remote_candidates = Vec::new();
        let mut remote_end_of_candidates_seen = false;
        let mut submitted_local_candidates = false;
        let mut submitted_local_end_of_candidates = false;
        let mut last_progress_at_ms = exchange_started_at_ms;
        let mut last_local_gathering_complete = false;
        let mut local_candidates_stable_since_ms: Option<f64> = None;

        // 第一批候选单独前置提交，避免 gathering 状态先行收敛把首轮 ICE 交换吞掉。
        let initial_local_candidates_batch = self.collect_unsent_local_candidates(
            &offer_sdp_candidates,
            &initial_local_candidates,
            &mut sent_local_candidates,
        )?;
        crate::xbx_log_warn!(
            "[xbxengine][runtime][ice] initial submit batch candidates={} summary={} offer_candidates={} initial_candidates={}",
            initial_local_candidates_batch.len(),
            Self::summarize_ice_candidate_kinds(&initial_local_candidates_batch),
            offer_sdp_candidates.len(),
            initial_local_candidates.len(),
        );
        if !initial_local_candidates_batch.is_empty() {
            crate::xbx_log_warn!(
                "[xbxengine][runtime][ice] submitting initial local candidates restart={restart}"
            );
            self.emit_phase(XbxEngineRuntimePhaseDto::ExchangingIce);
            self.ensure_operation_active(operation_epoch)?;
            Self::extract_submit_ice_response(self.host_bridge.request(
                XbxEngineHostRequestDto::SubmitIce {
                    session_id: self.require_session_id()?,
                    candidates: initial_local_candidates_batch,
                    restart,
                },
            )?)?;
            submitted_local_candidates = true;
            last_progress_at_ms = now_ms_f64();
        }

        loop {
            self.ensure_operation_active(operation_epoch)?;
            let mut outbound_local_candidates = self.collect_unsent_local_candidates(
                &offer_sdp_candidates,
                &initial_local_candidates,
                &mut sent_local_candidates,
            )?;
            let has_new_local_candidates = !outbound_local_candidates.is_empty();
            let now_ms = now_ms_f64();
            if has_new_local_candidates {
                local_candidates_stable_since_ms = None;
            } else if submitted_local_candidates {
                local_candidates_stable_since_ms.get_or_insert(now_ms);
            }
            let local_candidates_stable_elapsed_ms = local_candidates_stable_since_ms
                .map(|stable_since_ms| (now_ms - stable_since_ms).max(0.0));
            let mut appended_local_end_of_candidates = false;
            if should_submit_controlled_local_end_of_candidates(
                submitted_local_candidates,
                submitted_local_end_of_candidates,
                remote_end_of_candidates_seen,
                has_new_local_candidates,
                local_candidates_stable_elapsed_ms,
            ) {
                let local_end_of_candidates = build_local_end_of_candidates_candidate();
                let key = ice_candidate_dedupe_key(&local_end_of_candidates);
                if sent_local_candidates.insert(key) {
                    outbound_local_candidates.push(local_end_of_candidates);
                    appended_local_end_of_candidates = true;
                }
            }
            let local_gathering_complete = self.media_backend.local_ice_gathering_complete()?;
            if local_gathering_complete && !last_local_gathering_complete {
                last_local_gathering_complete = true;
                last_progress_at_ms = now_ms_f64();
            }
            crate::xbx_log_warn!(
                "[xbxengine][runtime][ice] exchange loop local_candidates={} submitted={} local_gathering_complete={} remote_eoc_seen={} local_eoc_submitted={} stable_elapsed_ms={:.0} remote_accumulated={}",
                outbound_local_candidates.len(),
                submitted_local_candidates,
                local_gathering_complete,
                remote_end_of_candidates_seen,
                submitted_local_end_of_candidates,
                local_candidates_stable_elapsed_ms.unwrap_or(0.0),
                aggregated_remote_candidates.len(),
            );

            if !outbound_local_candidates.is_empty() {
                if !submitted_local_candidates {
                    self.emit_phase(XbxEngineRuntimePhaseDto::ExchangingIce);
                }
                // 先把当前批次真实候选送出去，再继续轮询后续 trickle。
                self.ensure_operation_active(operation_epoch)?;
                crate::xbx_log_warn!(
                    "[xbxengine][runtime][ice] submitting local candidates batch size={} summary={} restart={restart}",
                    outbound_local_candidates.len(),
                    Self::summarize_ice_candidate_kinds(&outbound_local_candidates),
                );
                Self::extract_submit_ice_response(self.host_bridge.request(
                    XbxEngineHostRequestDto::SubmitIce {
                        session_id: self.require_session_id()?,
                        candidates: outbound_local_candidates,
                        restart,
                    },
                )?)?;
                submitted_local_candidates = true;
                if appended_local_end_of_candidates {
                    submitted_local_end_of_candidates = true;
                }
                last_progress_at_ms = now_ms_f64();
            } else if !submitted_local_candidates {
                if local_gathering_complete {
                    crate::xbx_log_warn!(
                        "[xbxengine][runtime][ice] skip initial submit because gathering completed before first batch"
                    );
                    break;
                }
                crate::xbx_log_warn!(
                    "[xbxengine][runtime][ice] waiting for first local candidate batch"
                );
                std::thread::sleep(Duration::from_millis(60));
                continue;
            }

            self.ensure_operation_active(operation_epoch)?;
            crate::xbx_log_warn!(
                "[xbxengine][runtime][ice] polling remote candidates restart={restart}"
            );
            let remote_candidates = Self::extract_poll_ice_response(self.host_bridge.request(
                XbxEngineHostRequestDto::PollIce {
                    session_id: self.require_session_id()?,
                    restart,
                },
            )?)?;
            crate::xbx_log_warn!(
                "[xbxengine][runtime][ice] polled remote candidates count={} summary={}",
                remote_candidates.len(),
                Self::summarize_ice_candidate_kinds(&remote_candidates),
            );
            self.ensure_operation_active(operation_epoch)?;
            let had_remote_end_of_candidates = remote_end_of_candidates_seen;
            remote_end_of_candidates_seen |= remote_candidates
                .iter()
                .any(|candidate| is_end_of_candidates_marker(&candidate.candidate));
            if remote_end_of_candidates_seen && !had_remote_end_of_candidates {
                last_progress_at_ms = now_ms_f64();
            }
            if remote_end_of_candidates_seen {
                crate::xbx_log_warn!("[xbxengine][runtime][ice] remote end-of-candidates observed");
            }

            let next_remote_candidates =
                dedupe_remote_ice_candidates(remote_candidates, &mut applied_remote_candidates);
            if !next_remote_candidates.is_empty() {
                let applied_batch_len = next_remote_candidates.len();
                self.media_backend
                    .add_remote_ice_candidates(next_remote_candidates.clone())?;
                aggregated_remote_candidates.extend(next_remote_candidates);
                last_progress_at_ms = now_ms_f64();
                crate::xbx_log_warn!(
                    "[xbxengine][runtime][ice] applied remote candidates batch size={} summary={} accumulated={}",
                    applied_batch_len,
                    Self::summarize_ice_candidate_kinds(&aggregated_remote_candidates),
                    aggregated_remote_candidates.len(),
                );
            }

            self.sync_runtime_activity_snapshot();

            if self.health.connected_at_ms.is_some() {
                crate::xbx_log_warn!(
                    "[xbxengine][runtime][ice] exchange loop exit because transport connected local_summary={} remote_summary={}",
                    Self::summarize_ice_candidate_kinds_from_set(&sent_local_candidates),
                    Self::summarize_ice_candidate_kinds(&aggregated_remote_candidates),
                );
                break;
            }
            if submitted_local_candidates {
                let exchange_elapsed_ms = now_ms_f64() - exchange_started_at_ms;
                let idle_elapsed_ms = now_ms_f64() - last_progress_at_ms;
                if should_allow_stable_exchange_settle(
                    submitted_local_candidates,
                    remote_end_of_candidates_seen,
                    has_new_local_candidates,
                    local_candidates_stable_elapsed_ms,
                ) {
                    crate::xbx_log_warn!(
                        "[xbxengine][runtime][ice] exchange loop exit because candidates settled local_summary={} remote_summary={} local_gathering_complete={} stable_elapsed_ms={:.0}",
                        Self::summarize_ice_candidate_kinds_from_set(&sent_local_candidates),
                        Self::summarize_ice_candidate_kinds(&aggregated_remote_candidates),
                        local_gathering_complete,
                        local_candidates_stable_elapsed_ms.unwrap_or(0.0),
                    );
                    break;
                }
                if local_gathering_complete && remote_end_of_candidates_seen {
                    crate::xbx_log_warn!(
                        "[xbxengine][runtime][ice] exchange loop exit because gathering complete and remote eoc seen local_summary={} remote_summary={}",
                        Self::summarize_ice_candidate_kinds_from_set(&sent_local_candidates),
                        Self::summarize_ice_candidate_kinds(&aggregated_remote_candidates),
                    );
                    break;
                }
                if exchange_elapsed_ms >= exchange_timeout_ms {
                    crate::xbx_log_warn!(
                        "[xbxengine][runtime][ice] exchange loop exit because timeout elapsed local_summary={} remote_summary={} idle_elapsed_ms={:.0} elapsed_ms={:.0}",
                        Self::summarize_ice_candidate_kinds_from_set(&sent_local_candidates),
                        Self::summarize_ice_candidate_kinds(&aggregated_remote_candidates),
                        idle_elapsed_ms,
                        exchange_elapsed_ms,
                    );
                    break;
                }
            } else if local_gathering_complete && remote_end_of_candidates_seen {
                crate::xbx_log_warn!(
                    "[xbxengine][runtime][ice] exchange loop exit because gathering complete and remote eoc seen before submit local_summary={} remote_summary={}",
                    Self::summarize_ice_candidate_kinds_from_set(&sent_local_candidates),
                    Self::summarize_ice_candidate_kinds(&aggregated_remote_candidates),
                );
                break;
            }
        }

        crate::xbx_log_warn!(
            "[xbxengine][runtime][ice] exchange loop finished local_summary={} remote_summary={} remote_total={}",
            Self::summarize_ice_candidate_kinds_from_set(&sent_local_candidates),
            Self::summarize_ice_candidate_kinds(&aggregated_remote_candidates),
            aggregated_remote_candidates.len(),
        );
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
        local_offer_candidates: &[XbxEngineIceCandidateDto],
        initial_local_candidates: &[XbxEngineIceCandidateDto],
        sent_local_candidates: &mut std::collections::HashSet<String>,
    ) -> Result<Vec<XbxEngineIceCandidateDto>, XbxEngineRuntimeError> {
        let mut pending = Vec::new();
        for candidate in local_offer_candidates
            .iter()
            .cloned()
            .chain(initial_local_candidates.iter().cloned())
            .chain(self.media_backend.local_candidates_snapshot()?.into_iter())
        {
            let key = ice_candidate_dedupe_key(&candidate);
            if sent_local_candidates.insert(key) {
                pending.push(candidate);
            }
        }
        Ok(pending)
    }

    fn summarize_ice_candidate_kinds(candidates: &[XbxEngineIceCandidateDto]) -> String {
        let mut host = 0usize;
        let mut srflx = 0usize;
        let mut relay = 0usize;
        let mut unknown = 0usize;
        let mut eoc = 0usize;
        for candidate in candidates {
            let trimmed = candidate.candidate.trim();
            if trimmed.eq_ignore_ascii_case("a=end-of-candidates")
                || trimmed.eq_ignore_ascii_case("end-of-candidates")
            {
                eoc += 1;
                continue;
            }
            match Self::parse_candidate_kind(trimmed) {
                "host" => host += 1,
                "srflx" => srflx += 1,
                "relay" => relay += 1,
                _ => unknown += 1,
            }
        }
        format!(
            "host={} srflx={} relay={} unknown={} eoc={}",
            host, srflx, relay, unknown, eoc
        )
    }

    fn summarize_ice_candidate_kinds_from_set(
        candidate_keys: &std::collections::HashSet<String>,
    ) -> String {
        let mut host = 0usize;
        let mut srflx = 0usize;
        let mut relay = 0usize;
        let mut unknown = 0usize;
        let mut eoc = 0usize;
        for key in candidate_keys {
            let candidate = key.split('|').next().unwrap_or_default().trim();
            if candidate.eq_ignore_ascii_case("a=end-of-candidates")
                || candidate.eq_ignore_ascii_case("end-of-candidates")
            {
                eoc += 1;
                continue;
            }
            match Self::parse_candidate_kind(candidate) {
                "host" => host += 1,
                "srflx" => srflx += 1,
                "relay" => relay += 1,
                _ => unknown += 1,
            }
        }
        format!(
            "host={} srflx={} relay={} unknown={} eoc={}",
            host, srflx, relay, unknown, eoc
        )
    }

    fn parse_candidate_kind(candidate: &str) -> &'static str {
        let mut tokens = candidate
            .split_whitespace()
            .map(|token| token.to_ascii_lowercase());
        while let Some(token) = tokens.next() {
            if token == "typ" {
                return match tokens.next().as_deref() {
                    Some("host") => "host",
                    Some("srflx") => "srflx",
                    Some("relay") => "relay",
                    _ => "unknown",
                };
            }
        }
        "unknown"
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

fn resolve_ice_exchange_timeout_ms(first_frame_grace_ms: u64, reconnect_stall_ms: u64) -> f64 {
    // 临时 A/B：把 exchange timeout 固定约束在 [10s, 12s]，用于验证是否存在“超时过短误杀”。
    let baseline = first_frame_grace_ms
        .max(reconnect_stall_ms)
        .max(ICE_EXCHANGE_TIMEOUT_MS_MIN as u64);
    baseline.min(ICE_EXCHANGE_TIMEOUT_MS_MAX as u64) as f64
}

fn should_allow_stable_exchange_settle(
    submitted_local_candidates: bool,
    remote_end_of_candidates_seen: bool,
    has_new_local_candidates: bool,
    local_candidates_stable_elapsed_ms: Option<f64>,
) -> bool {
    submitted_local_candidates
        && remote_end_of_candidates_seen
        && !has_new_local_candidates
        && local_candidates_stable_elapsed_ms
            .is_some_and(|elapsed| elapsed >= ICE_EXCHANGE_STABLE_SETTLE_WINDOW_MS)
}

fn should_submit_controlled_local_end_of_candidates(
    submitted_local_candidates: bool,
    submitted_local_end_of_candidates: bool,
    remote_end_of_candidates_seen: bool,
    has_new_local_candidates: bool,
    local_candidates_stable_elapsed_ms: Option<f64>,
) -> bool {
    !submitted_local_end_of_candidates
        && should_allow_stable_exchange_settle(
            submitted_local_candidates,
            remote_end_of_candidates_seen,
            has_new_local_candidates,
            local_candidates_stable_elapsed_ms,
        )
}

fn build_local_end_of_candidates_candidate() -> XbxEngineIceCandidateDto {
    XbxEngineIceCandidateDto {
        candidate: "a=end-of-candidates".to_string(),
        sdp_m_line_index: Some(0),
        sdp_mid: Some("0".to_string()),
    }
}

fn runtime_stats_indicate_transport_recovering(
    runtime_stats: &crate::XbxEngineMediaRuntimeStats,
) -> bool {
    if runtime_stats.transport_state != XbxEngineTransportStateDto::Connecting {
        return false;
    }
    runtime_stats
        .latest_observation_summary
        .as_deref()
        .is_some_and(|summary| summary.contains("recoveryActionCreated=true"))
        || runtime_stats
            .latest_observation_label
            .as_deref()
            .is_some_and(|label| {
                matches!(
                    label,
                    "rtcConnectionRecovering"
                        | "rtcPeerConnectionFailed"
                        | "rtcPeerConnectionClosed"
                        | "rtcControlChannelClosed"
                        | "rtcMessageChannelClosed"
                )
            })
}

fn is_stop_gamepad_rumble_request(effect: &OhMyGamepadRumbleEffectDto) -> bool {
    effect.duration_ms == 0
        && effect.start_delay_ms == 0
        && effect.repeat == 0
        && effect.strong_magnitude == 0.0
        && effect.weak_magnitude == 0.0
        && effect.left_trigger == 0.0
        && effect.right_trigger == 0.0
}

fn collect_local_offer_ice_candidates(offer_sdp: &str) -> Vec<XbxEngineIceCandidateDto> {
    let mut candidates = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut seen_media_line = false;
    let mut current_mline_index = 0u16;
    let mut current_mid: Option<String> = None;

    // 优先复用 offer 中已有 candidate，避免启动阶段卡在候选快照尚未刷新。
    for line in offer_sdp.lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        if line.starts_with("m=") {
            if seen_media_line {
                current_mline_index = current_mline_index.saturating_add(1);
            } else {
                seen_media_line = true;
            }
            current_mid = None;
            continue;
        }
        if let Some(mid) = line.strip_prefix("a=mid:") {
            let mid = mid.trim();
            if !mid.is_empty() {
                current_mid = Some(mid.to_string());
            }
            continue;
        }
        if !line.starts_with("a=candidate:") && !line.starts_with("candidate:") {
            continue;
        }

        let candidate = XbxEngineIceCandidateDto {
            candidate: line.to_string(),
            sdp_m_line_index: Some(current_mline_index),
            sdp_mid: current_mid.clone(),
        };
        let key = ice_candidate_dedupe_key(&candidate);
        if seen.insert(key) {
            candidates.push(candidate);
        }
    }

    candidates
}

fn normalize_offer_profile_token(profile: &str) -> String {
    let normalized = profile.trim().to_ascii_lowercase();
    let normalized = normalized
        .strip_prefix("profile-level-id=")
        .unwrap_or(normalized.as_str())
        .to_string();
    match normalized.as_str() {
        "high" | "main" => "4d".to_string(),
        "normal" | "default" | "browser" | "macos" | "rust-owned" => "42e".to_string(),
        "low" | "baseline" => "420".to_string(),
        other => other.to_string(),
    }
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

fn is_control_channel_not_ready_error(error: &XbxEngineRuntimeError) -> bool {
    let normalized = error.to_string().to_ascii_lowercase();
    normalized.contains("xbxenginertccontrolchannelnotreadyforkeyframe")
        || normalized.contains("xbxenginertccontrolchannelnotreadyfordecoderreset")
}

#[cfg(test)]
mod tests {
    use super::{
        build_local_end_of_candidates_candidate, collect_local_offer_ice_candidates,
        resolve_ice_exchange_timeout_ms, should_allow_stable_exchange_settle,
        should_submit_controlled_local_end_of_candidates, ICE_EXCHANGE_STABLE_SETTLE_WINDOW_MS,
    };

    #[test]
    fn collects_offer_sdp_candidates_from_realistic_sdp() {
        let offer = concat!(
            "v=0\r\n",
            "o=- 3626176003912642578 968255000 IN IP4 0.0.0.0\r\n",
            "s=-\r\n",
            "t=0 0\r\n",
            "m=audio 9 UDP/TLS/RTP/SAVPF 111 9 0 8\r\n",
            "a=mid:0\r\n",
            "a=candidate:2067167569 1 udp 2130706431 fdfe:dcba:9876::1 63405 typ host\r\n",
            "a=candidate:2067167569 2 udp 2130706431 fdfe:dcba:9876::1 63405 typ host\r\n",
            "m=video 9 UDP/TLS/RTP/SAVPF 123 124 125\r\n",
            "a=mid:1\r\n",
            "a=candidate:2067167569 1 udp 2130706431 fdfe:dcba:9876::1 63405 typ host\r\n",
        );

        let candidates = collect_local_offer_ice_candidates(offer);
        assert_eq!(candidates.len(), 3);
        assert_eq!(candidates[0].sdp_mid.as_deref(), Some("0"));
        assert_eq!(candidates[0].sdp_m_line_index, Some(0));
        assert_eq!(candidates[2].sdp_mid.as_deref(), Some("1"));
        assert_eq!(candidates[2].sdp_m_line_index, Some(1));
    }

    #[test]
    fn ice_exchange_timeout_uses_bounded_floor() {
        assert_eq!(resolve_ice_exchange_timeout_ms(1_800, 2_400), 10_000.0);
        assert_eq!(resolve_ice_exchange_timeout_ms(8_000, 4_000), 10_000.0);
        assert_eq!(resolve_ice_exchange_timeout_ms(15_000, 8_000), 12_000.0);
    }

    #[test]
    fn stable_exchange_settle_does_not_require_local_gathering_complete() {
        assert!(should_allow_stable_exchange_settle(
            true,
            true,
            false,
            Some(ICE_EXCHANGE_STABLE_SETTLE_WINDOW_MS + 1.0),
        ));
    }

    #[test]
    fn controlled_local_eoc_is_single_shot_after_stable_window() {
        assert!(should_submit_controlled_local_end_of_candidates(
            true,
            false,
            true,
            false,
            Some(ICE_EXCHANGE_STABLE_SETTLE_WINDOW_MS + 10.0),
        ));
        assert!(!should_submit_controlled_local_end_of_candidates(
            true,
            true,
            true,
            false,
            Some(ICE_EXCHANGE_STABLE_SETTLE_WINDOW_MS + 10.0),
        ));
    }

    #[test]
    fn stable_exchange_settle_requires_remote_eoc_and_stable_window() {
        assert!(!should_allow_stable_exchange_settle(
            true,
            false,
            false,
            Some(ICE_EXCHANGE_STABLE_SETTLE_WINDOW_MS + 100.0),
        ));
        assert!(!should_allow_stable_exchange_settle(
            true,
            true,
            false,
            Some(ICE_EXCHANGE_STABLE_SETTLE_WINDOW_MS - 1.0),
        ));
    }

    #[test]
    fn local_end_of_candidates_marker_uses_expected_contract() {
        let marker = build_local_end_of_candidates_candidate();
        assert_eq!(marker.candidate, "a=end-of-candidates");
        assert_eq!(marker.sdp_m_line_index, Some(0));
        assert_eq!(marker.sdp_mid.as_deref(), Some("0"));
    }
}
