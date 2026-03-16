use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};
use xbxengine::{
    create_active_media_backend, OhMyGamepadXbxEngineInputBackend, XbxEngineEventSink,
    XbxEngineHostBridge, XbxEngineMediaBackend, XbxEngineRuntime, XbxEngineRuntimeConfig,
    XbxEngineRuntimeError,
};
use xbxengine_protocol::XbxEngineStatsDto;
use xbxengine_protocol::{
    XbxEngineControlCommandDto, XbxEngineHostRequestDto, XbxEngineHostResponseDto,
    XbxEngineIceCandidateDto, XbxEngineRuntimeEventDto,
};

use crate::error::{AppError, AppResult};
use crate::mods::native_video::NativeVideoRegistryRef;
use crate::mods::runtime_trace::RuntimeTraceRecorderRef;
use crate::mods::streaming::{
    StreamingCloseSessionParams, StreamingExchangeOfferParams, StreamingPollIceParams,
    StreamingSubmitIceParams,
};
use crate::shell::bridge::{TauriEngineEventBridge, TauriEngineWindowHost};
use crate::AppState;

type TauriXbxEngineRuntime = XbxEngineRuntime<
    TauriXbxEngineHostBridge,
    TauriXbxEngineEventSink,
    Box<dyn XbxEngineMediaBackend>,
>;

/// 运行态只负责持有 runtime 实例和并发访问。
pub struct XbxEngineRuntimeState {
    runtime: StdMutex<TauriXbxEngineRuntime>,
    runtime_trace: RuntimeTraceRecorderRef,
    last_stats_trace_at: StdMutex<Option<Instant>>,
    last_trace_observation: StdMutex<RuntimeTraceObservationState>,
    active_session_id: StdMutex<Option<String>>,
    cancellation_epoch: Arc<AtomicU64>,
}

#[derive(Default)]
struct RuntimeTraceObservationState {
    packet_gap_observation_id: Option<u64>,
    frame_drop_observation_id: Option<u64>,
    nack_observation_id: Option<u64>,
    escalation_observation_id: Option<u64>,
    bwe_observation_id: Option<u64>,
    twcc_observation_id: Option<u64>,
    recovery_keyframe_request_count: Option<u64>,
    recovery_decoder_reset_count: Option<u64>,
    recovery_reconnect_count: Option<u64>,
    transport_state: Option<String>,
    transport_path: Option<String>,
    video_remb_bps: Option<u32>,
}

impl XbxEngineRuntimeState {
    pub fn new(
        app_handle: AppHandle,
        last_runtime_event: Arc<StdMutex<Option<serde_json::Value>>>,
        native_video: NativeVideoRegistryRef,
        runtime_trace: RuntimeTraceRecorderRef,
    ) -> Self {
        let cancellation_epoch = Arc::new(AtomicU64::new(0));
        let event_bridge = TauriEngineEventBridge {
            app_handle: app_handle.clone(),
            state: Default::default(),
            last_runtime_event,
            runtime_trace: runtime_trace.clone(),
        };
        let input_backend = Box::new(OhMyGamepadXbxEngineInputBackend::new());
        let media_backend =
            create_active_media_backend(input_backend, XbxEngineRuntimeConfig::default());
        let runtime = XbxEngineRuntime::with_media_backend(
            XbxEngineRuntimeConfig::default(),
            TauriXbxEngineHostBridge {
                app_handle,
                native_video,
                runtime_trace: runtime_trace.clone(),
                cancellation_epoch: cancellation_epoch.clone(),
            },
            TauriXbxEngineEventSink {
                bridge: Arc::new(StdMutex::new(event_bridge)),
            },
            media_backend,
        );
        Self {
            runtime: StdMutex::new(runtime),
            runtime_trace,
            last_stats_trace_at: StdMutex::new(None),
            last_trace_observation: StdMutex::new(RuntimeTraceObservationState::default()),
            active_session_id: StdMutex::new(None),
            cancellation_epoch,
        }
    }

    pub fn apply_control(
        &self,
        command: XbxEngineControlCommandDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        let command_value = serde_json::to_value(&command).unwrap_or(serde_json::Value::Null);
        let session_id = extract_command_session_id(&command);
        self.runtime_trace.record_value(
            "xbxengine",
            "controlCommand",
            session_id.as_deref(),
            command_value,
        );
        match &command {
            // Stop 需要先发出取消信号，打断正在路上的 keepalive/offer/ice。
            XbxEngineControlCommandDto::StopRuntime => {
                self.cancellation_epoch.fetch_add(1, Ordering::SeqCst);
            }
            XbxEngineControlCommandDto::StartRuntime { .. } => {}
            _ => {}
        }
        if let Ok(mut active_session_id) = self.active_session_id.lock() {
            match &command {
                XbxEngineControlCommandDto::StartRuntime { session, .. } => {
                    *active_session_id = Some(session.session_id.clone());
                }
                XbxEngineControlCommandDto::StopRuntime => {
                    *active_session_id = None;
                }
                _ => {}
            }
        }
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRuntimeLockPoisoned"))?;
        runtime.apply_control(command)
    }

    pub fn tick(&self) -> Result<(), XbxEngineRuntimeError> {
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRuntimeLockPoisoned"))?;
        runtime.tick();
        let stats_snapshot = runtime.snapshot_stats();
        let session_id = self
            .active_session_id
            .lock()
            .ok()
            .and_then(|guard| guard.clone());
        if should_skip_trace_tick(session_id.as_deref(), &stats_snapshot) {
            return Ok(());
        }
        self.record_runtime_trace_observations(session_id.as_deref(), &stats_snapshot);
        let Ok(mut last_stats_trace_at) = self.last_stats_trace_at.lock() else {
            return Ok(());
        };
        let now = Instant::now();
        let should_record = last_stats_trace_at
            .map(|last| now.duration_since(last) >= Duration::from_secs(1))
            .unwrap_or(true);
        if should_record {
            *last_stats_trace_at = Some(now);
            if let Ok(snapshot) = serde_json::to_value(&stats_snapshot) {
                self.runtime_trace.record_value(
                    "xbxengine",
                    "statsSnapshot",
                    session_id.as_deref(),
                    snapshot,
                );
            }
        }
        Ok(())
    }

    pub fn snapshot_stats(&self) -> AppResult<serde_json::Value> {
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| AppError::XbxEngine("Failed to lock xbxengine runtime".to_string()))?;
        Ok(serde_json::to_value(runtime.snapshot_stats())?)
    }

    fn record_runtime_trace_observations(
        &self,
        session_id: Option<&str>,
        stats: &XbxEngineStatsDto,
    ) {
        let Ok(mut observation_state) = self.last_trace_observation.lock() else {
            return;
        };

        if let Some(packet_gap) = stats.latest_video_packet_gap.as_ref() {
            if observation_state.packet_gap_observation_id != Some(packet_gap.observation_id) {
                observation_state.packet_gap_observation_id = Some(packet_gap.observation_id);
                self.runtime_trace.record(
                    "xbxengine",
                    "packetGapDetected",
                    session_id,
                    serde_json::json!({
                        "observationId": packet_gap.observation_id,
                        "expectedSequence": packet_gap.expected_sequence,
                        "receivedSequence": packet_gap.received_sequence,
                        "missingCount": packet_gap.missing_count,
                        "source": packet_gap.source,
                        "frameRtpTimestamp": packet_gap.frame_rtp_timestamp,
                        "framePacketCount": packet_gap.frame_packet_count,
                        "frameMissingCount": packet_gap.frame_missing_count,
                        "frameIsKeyframe": packet_gap.frame_is_keyframe,
                        "frameImportance": packet_gap.frame_importance,
                        "observedAtMs": packet_gap.observed_at_ms,
                    }),
                );
            }
        }

        if let Some(frame_drop) = stats.latest_video_frame_drop.as_ref() {
            if observation_state.frame_drop_observation_id != Some(frame_drop.observation_id) {
                observation_state.frame_drop_observation_id = Some(frame_drop.observation_id);
                let event_name = if frame_drop.reason == "dropLate" {
                    "frameDeadlineMissed"
                } else {
                    "frameDropped"
                };
                self.runtime_trace.record(
                    "xbxengine",
                    event_name,
                    session_id,
                    serde_json::json!({
                        "observationId": frame_drop.observation_id,
                        "reason": frame_drop.reason,
                        "observedAtMs": frame_drop.observed_at_ms,
                        "width": frame_drop.width,
                        "height": frame_drop.height,
                        "isKeyframe": frame_drop.is_keyframe,
                        "queueDepth": frame_drop.queue_depth,
                    }),
                );
            }
        }

        if let Some(nack) = stats.latest_video_nack_observation.as_ref() {
            if observation_state.nack_observation_id != Some(nack.observation_id) {
                observation_state.nack_observation_id = Some(nack.observation_id);
                let event_name = match nack.action.as_str() {
                    "expiredDeadline" | "expiredMaxAge" => "nackExpired",
                    "recovered" | "recoveredLate" => "nackRecovered",
                    _ => "nackSent",
                };
                self.runtime_trace.record(
                    "xbxengine",
                    event_name,
                    session_id,
                    serde_json::json!({
                        "observationId": nack.observation_id,
                        "action": nack.action,
                        "source": nack.source,
                        "firstSequence": nack.first_sequence,
                        "lastSequence": nack.last_sequence,
                        "packetCount": nack.packet_count,
                        "retryCount": nack.retry_count,
                        "frameRtpTimestamp": nack.frame_rtp_timestamp,
                        "frameIsKeyframe": nack.frame_is_keyframe,
                        "frameImportance": nack.frame_importance,
                        "deadlineAtMs": nack.deadline_at_ms,
                        "observedAtMs": nack.observed_at_ms,
                    }),
                );
            }
        }

        if let Some(escalation) = stats.latest_video_escalation_observation.as_ref() {
            if observation_state.escalation_observation_id != Some(escalation.observation_id) {
                observation_state.escalation_observation_id = Some(escalation.observation_id);
                self.runtime_trace.record(
                    "xbxengine",
                    "videoEscalation",
                    session_id,
                    serde_json::json!({
                        "observationId": escalation.observation_id,
                        "reason": escalation.reason,
                        "action": escalation.action,
                        "observedAtMs": escalation.observed_at_ms,
                    }),
                );
            }
        }

        if let Some(bwe) = stats.latest_video_bwe_observation.as_ref() {
            if observation_state.bwe_observation_id != Some(bwe.observation_id) {
                observation_state.bwe_observation_id = Some(bwe.observation_id);
                self.runtime_trace.record(
                    "xbxengine",
                    "bweUpdated",
                    session_id,
                    serde_json::json!({
                        "observationId": bwe.observation_id,
                        "mode": bwe.mode,
                        "decisionReason": bwe.decision_reason,
                        "targetRembKbps": bwe.target_remb_kbps,
                        "observedRembKbps": bwe.observed_remb_kbps,
                        "actualVideoBitrateKbps": bwe.actual_video_bitrate_kbps,
                        "lossRatio": bwe.loss_ratio,
                        "rttMs": bwe.rtt_ms,
                        "transportPath": bwe.transport_path,
                        "twccFeedbackIntervalMs": bwe.twcc_feedback_interval_ms,
                        "twccObservedPacketCount": bwe.twcc_observed_packet_count,
                        "twccCoveredSequenceSpan": bwe.twcc_covered_sequence_span,
                        "twccReceiveBitrateKbps": bwe.twcc_receive_bitrate_kbps,
                        "twccDeliveryRatio": bwe.twcc_delivery_ratio,
                        "twccLossRatio": bwe.twcc_loss_ratio,
                        "observedAtMs": bwe.observed_at_ms,
                    }),
                );
            }
        }

        if let Some(twcc) = stats.latest_video_twcc_observation.as_ref() {
            if observation_state.twcc_observation_id != Some(twcc.observation_id) {
                observation_state.twcc_observation_id = Some(twcc.observation_id);
                self.runtime_trace.record(
                    "xbxengine",
                    "twccFeedbackSent",
                    session_id,
                    serde_json::json!({
                        "observationId": twcc.observation_id,
                        "feedbackPacketCount": twcc.feedback_packet_count,
                        "coveredSequenceStart": twcc.covered_sequence_start,
                        "coveredSequenceEnd": twcc.covered_sequence_end,
                        "coveredSequenceSpan": twcc.covered_sequence_span,
                        "observedPacketCount": twcc.observed_packet_count,
                        "observedByteCount": twcc.observed_byte_count,
                        "feedbackIntervalMs": twcc.feedback_interval_ms,
                        "arrivalSpanMs": twcc.arrival_span_ms,
                        "receiveBitrateKbps": twcc.receive_bitrate_kbps,
                        "deliveryRatio": twcc.delivery_ratio,
                        "packetLossRatio": twcc.packet_loss_ratio,
                        "observedAtMs": twcc.observed_at_ms,
                    }),
                );
            }
        }

        if observation_state.recovery_keyframe_request_count
            != stats.recovery_keyframe_request_count
        {
            observation_state.recovery_keyframe_request_count =
                stats.recovery_keyframe_request_count;
            if let Some(count) = stats.recovery_keyframe_request_count {
                if count > 0 && stats.last_recovery_action.as_deref() == Some("keyframe") {
                    self.runtime_trace.record(
                        "xbxengine",
                        "keyframeRequested",
                        session_id,
                        serde_json::json!({
                            "count": count,
                            "atMs": stats.last_recovery_action_at_ms,
                            "reason": stats.last_recovery_reason,
                        }),
                    );
                }
            }
        }

        if observation_state.recovery_decoder_reset_count != stats.recovery_decoder_reset_count {
            observation_state.recovery_decoder_reset_count = stats.recovery_decoder_reset_count;
            if let Some(count) = stats.recovery_decoder_reset_count {
                if count > 0 && stats.last_recovery_action.as_deref() == Some("decoderReset") {
                    self.runtime_trace.record(
                        "xbxengine",
                        "decoderResetRequested",
                        session_id,
                        serde_json::json!({
                            "count": count,
                            "atMs": stats.last_recovery_action_at_ms,
                            "reason": stats.last_recovery_reason,
                        }),
                    );
                }
            }
        }

        if observation_state.recovery_reconnect_count != stats.recovery_reconnect_count {
            observation_state.recovery_reconnect_count = stats.recovery_reconnect_count;
        }

        if observation_state.transport_state != stats.transport_state
            || observation_state.transport_path != stats.transport_path
        {
            observation_state.transport_state = stats.transport_state.clone();
            observation_state.transport_path = stats.transport_path.clone();
            self.runtime_trace.record(
                "xbxengine",
                "transportObservation",
                session_id,
                serde_json::json!({
                    "transportState": stats.transport_state,
                    "transportPath": stats.transport_path,
                }),
            );
        }

        if observation_state.video_remb_bps != stats.video_remb_bps {
            observation_state.video_remb_bps = stats.video_remb_bps;
            if let Some(video_remb_bps) = stats.video_remb_bps {
                self.runtime_trace.record(
                    "xbxengine",
                    "rembUpdated",
                    session_id,
                    serde_json::json!({
                        "videoRembBps": video_remb_bps,
                    }),
                );
            }
        }
    }
}

#[derive(Clone)]
struct TauriXbxEngineHostBridge {
    app_handle: AppHandle,
    native_video: NativeVideoRegistryRef,
    runtime_trace: RuntimeTraceRecorderRef,
    cancellation_epoch: Arc<AtomicU64>,
}

impl TauriXbxEngineHostBridge {
    fn app_state(&self) -> AppResult<tauri::State<'_, AppState>> {
        self.app_handle.try_state::<AppState>().ok_or_else(|| {
            AppError::XbxEngine("AppState unavailable for xbxengine host bridge".to_string())
        })
    }

    fn exchange_offer(
        &self,
        session_id: String,
        channel: String,
        sdp: String,
        restart: bool,
    ) -> Result<XbxEngineHostResponseDto, XbxEngineRuntimeError> {
        let state = self.app_state().map_err(map_app_error("exchangeOffer"))?;
        self.runtime_trace.record(
            "xbxengine-host",
            "exchangeOfferRequested",
            Some(&session_id),
            serde_json::json!({
                "channel": channel,
                "restart": restart,
                "offerSdp": sdp,
            }),
        );
        let result = tauri::async_runtime::block_on(state.streaming.exchange_offer(
            StreamingExchangeOfferParams {
                session_id,
                channel: Some(channel),
                sdp,
                restart,
            },
        ))
        .map_err(map_app_error("exchangeOffer"))?;
        self.runtime_trace.record(
            "xbxengine-host",
            "exchangeOfferResult",
            None,
            serde_json::json!({
                "answerSdp": result.answer.sdp,
            }),
        );
        Ok(XbxEngineHostResponseDto::OfferExchanged {
            answer_sdp: result.answer.sdp,
        })
    }

    fn submit_ice(
        &self,
        session_id: String,
        candidates: Vec<XbxEngineIceCandidateDto>,
        restart: bool,
    ) -> Result<XbxEngineHostResponseDto, XbxEngineRuntimeError> {
        let state = self.app_state().map_err(map_app_error("submitIce"))?;
        let trace_session_id = session_id.clone();
        self.runtime_trace.record(
            "xbxengine-host",
            "submitIceRequested",
            Some(&session_id),
            serde_json::json!({
                "restart": restart,
                "candidates": candidates,
            }),
        );
        tauri::async_runtime::block_on(
            state.streaming.submit_ice(StreamingSubmitIceParams {
                session_id,
                candidate: candidates
                    .into_iter()
                    .map(|candidate| crate::mods::streaming::StreamingIceCandidate {
                        candidate: candidate.candidate,
                        sdp_m_line_index: candidate.sdp_m_line_index.map(u32::from),
                        sdp_mid: candidate.sdp_mid,
                        username_fragment: None,
                        message_type: None,
                    })
                    .collect(),
                restart,
            }),
        )
        .map_err(map_app_error("submitIce"))?;
        self.runtime_trace.record(
            "xbxengine-host",
            "submitIceResult",
            Some(&trace_session_id),
            serde_json::json!({
                "accepted": true,
                "restart": restart,
            }),
        );
        Ok(XbxEngineHostResponseDto::IceSubmitted)
    }

    fn poll_ice(
        &self,
        session_id: String,
        restart: bool,
    ) -> Result<XbxEngineHostResponseDto, XbxEngineRuntimeError> {
        let state = self.app_state().map_err(map_app_error("pollIce"))?;
        self.runtime_trace.record(
            "xbxengine-host",
            "pollIceRequested",
            Some(&session_id),
            serde_json::json!({
                "restart": restart,
            }),
        );
        let result =
            tauri::async_runtime::block_on(state.streaming.poll_ice(StreamingPollIceParams {
                session_id,
                restart,
            }))
            .map_err(map_app_error("pollIce"))?;
        self.runtime_trace.record(
            "xbxengine-host",
            "pollIceResult",
            None,
            serde_json::json!({
                "candidates": result.candidates,
            }),
        );
        Ok(XbxEngineHostResponseDto::IcePolled {
            candidates: result
                .candidates
                .into_iter()
                .map(|candidate| XbxEngineIceCandidateDto {
                    candidate: candidate.candidate,
                    sdp_m_line_index: candidate.sdp_m_line_index.map(|value| value as u16),
                    sdp_mid: candidate.sdp_mid,
                })
                .collect(),
        })
    }

    fn close_remote_session(
        &self,
        session_id: String,
    ) -> Result<XbxEngineHostResponseDto, XbxEngineRuntimeError> {
        let state = self
            .app_handle
            .try_state::<AppState>()
            .ok_or_else(|| XbxEngineRuntimeError::new("xbxEngineAppStateUnavailable"))?;
        self.runtime_trace.record(
            "xbxengine-host",
            "closeRemoteSessionRequested",
            Some(&session_id),
            serde_json::json!({}),
        );
        tauri::async_runtime::block_on(
            state
                .streaming
                .close_session(StreamingCloseSessionParams { session_id }),
        )
        .map_err(map_app_error("closeRemoteSession"))?;
        self.runtime_trace.record(
            "xbxengine-host",
            "closeRemoteSessionResult",
            None,
            serde_json::json!({ "closed": true }),
        );
        Ok(XbxEngineHostResponseDto::RemoteSessionClosed)
    }

    fn attach_native_viewport(
        &self,
        viewport_id: &str,
        surface_id: Option<&str>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let Ok(mut registry) = self.native_video.lock() else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineNativeVideoRegistryLockFailed",
            ));
        };
        let changed = registry.attach_viewport(viewport_id, surface_id);
        if changed {
            self.runtime_trace.record(
                "xbxengine-host",
                "nativeViewportAttached",
                None,
                serde_json::json!({
                    "viewportId": viewport_id,
                    "surfaceId": surface_id,
                }),
            );
        }
        Ok(())
    }

    fn detach_native_viewport(&self, viewport_id: &str) -> Result<(), XbxEngineRuntimeError> {
        let Ok(mut registry) = self.native_video.lock() else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineNativeVideoRegistryLockFailed",
            ));
        };
        registry.detach_viewport(viewport_id);
        self.runtime_trace.record(
            "xbxengine-host",
            "nativeViewportDetached",
            None,
            serde_json::json!({
                "viewportId": viewport_id,
            }),
        );
        Ok(())
    }

    fn present_native_frame(
        &self,
        viewport_id: &str,
        surface_id: Option<&str>,
        frame: &xbxengine::XbxEngineRenderFrame,
    ) -> Result<(), XbxEngineRuntimeError> {
        let Ok(mut registry) = self.native_video.lock() else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineNativeVideoRegistryLockFailed",
            ));
        };
        registry.present_frame(viewport_id, surface_id, frame);
        Ok(())
    }
}

impl XbxEngineHostBridge for TauriXbxEngineHostBridge {
    fn current_cancellation_epoch(&self) -> u64 {
        self.cancellation_epoch.load(Ordering::SeqCst)
    }

    fn attach_viewport(
        &mut self,
        viewport: &xbxengine_protocol::XbxEngineViewportDto,
        surface_id: Option<&str>,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.attach_native_viewport(&viewport.viewport_id, surface_id)
    }

    fn detach_viewport(&mut self, viewport_id: Option<&str>) -> Result<(), XbxEngineRuntimeError> {
        let Some(viewport_id) = viewport_id else {
            return Ok(());
        };
        self.detach_native_viewport(viewport_id)
    }

    fn present_frame(
        &mut self,
        viewport: &xbxengine_protocol::XbxEngineViewportDto,
        surface_id: Option<&str>,
        frame: &xbxengine::XbxEngineRenderFrame,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.present_native_frame(&viewport.viewport_id, surface_id, frame)
    }

    fn request(
        &mut self,
        request: XbxEngineHostRequestDto,
    ) -> Result<XbxEngineHostResponseDto, XbxEngineRuntimeError> {
        match request {
            XbxEngineHostRequestDto::ExchangeOffer {
                session_id,
                channel,
                sdp,
                restart,
            } => self.exchange_offer(session_id, channel, sdp, restart),
            XbxEngineHostRequestDto::SubmitIce {
                session_id,
                candidates,
                restart,
            } => self.submit_ice(session_id, candidates, restart),
            XbxEngineHostRequestDto::PollIce {
                session_id,
                restart,
            } => self.poll_ice(session_id, restart),
            XbxEngineHostRequestDto::KeepAliveRemoteSession { session_id } => {
                let state = self
                    .app_state()
                    .map_err(map_app_error("keepAliveRemoteSession"))?;
                tauri::async_runtime::block_on(state.streaming.send_keepalive(session_id))
                    .map_err(map_app_error("keepAliveRemoteSession"))?;
                self.runtime_trace.record(
                    "xbxengine-host",
                    "keepAliveRemoteSession",
                    None,
                    serde_json::json!({ "accepted": true }),
                );
                Ok(XbxEngineHostResponseDto::KeepAliveAccepted)
            }
            XbxEngineHostRequestDto::CloseRemoteSession { session_id, .. } => {
                self.close_remote_session(session_id)
            }
        }
    }
}

#[derive(Clone)]
struct TauriXbxEngineEventSink {
    bridge: Arc<StdMutex<TauriEngineEventBridge>>,
}

impl XbxEngineEventSink for TauriXbxEngineEventSink {
    fn emit(&mut self, event: XbxEngineRuntimeEventDto) {
        if let Ok(mut bridge) = self.bridge.lock() {
            bridge.apply_event(&event);
        }
    }
}

fn map_app_error(action: &'static str) -> impl FnOnce(AppError) -> XbxEngineRuntimeError {
    move |error| XbxEngineRuntimeError::new(format!("{action}:{error}"))
}

fn extract_command_session_id(command: &XbxEngineControlCommandDto) -> Option<String> {
    match command {
        XbxEngineControlCommandDto::StartRuntime { session, .. } => {
            Some(session.session_id.clone())
        }
        _ => None,
    }
}

fn should_skip_trace_tick(session_id: Option<&str>, stats: &XbxEngineStatsDto) -> bool {
    session_id.is_none() && stats.transport_state.as_deref() == Some("Closed")
}
