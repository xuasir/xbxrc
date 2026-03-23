use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use ohmygamepad_protocol::{
    OhMyGamepadRumbleRequestDto, OhMyGamepadRumbleResultDto, OhMyGamepadRumbleTargetDto,
};
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
use crate::mods::native_video::{NativeVideoRegistryRef, NativeVideoViewportState};
use crate::mods::runtime_trace::RuntimeTraceRecorderRef;
use crate::mods::streaming::{
    StreamingCloseSessionParams, StreamingExchangeOfferParams, StreamingPollIceParams,
    StreamingSubmitIceParams,
};
use crate::mods::xbxengine::trace_projection::{
    build_observability_snapshot, record_runtime_trace_observations, should_skip_trace_tick,
    RuntimeTraceObservationState,
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
    native_video: NativeVideoRegistryRef,
    runtime_trace: RuntimeTraceRecorderRef,
    last_stats_trace_at: StdMutex<Option<Instant>>,
    last_trace_observation: StdMutex<RuntimeTraceObservationState>,
    active_session_id: StdMutex<Option<String>>,
    cancellation_epoch: Arc<AtomicU64>,
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
                native_video: native_video.clone(),
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
            native_video,
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
        self.runtime_trace.record_event(
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
        let viewport_id = runtime
            .snapshot()
            .viewport
            .as_ref()
            .map(|viewport| viewport.viewport_id.clone());
        self.sync_native_video_host_timing(&mut runtime, viewport_id.as_deref());
        runtime.tick();
        let mut stats_snapshot = runtime.snapshot_stats();
        self.apply_native_video_host_stats(&mut stats_snapshot, viewport_id.as_deref());
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
                self.runtime_trace.record_snapshot(
                    "xbxengine",
                    "statsSnapshot",
                    session_id.as_deref(),
                    snapshot,
                );
            }
            self.runtime_trace.record_snapshot(
                "xbxengine",
                "observabilitySnapshot",
                session_id.as_deref(),
                build_observability_snapshot(&stats_snapshot),
            );
        }
        Ok(())
    }

    pub fn snapshot_stats(&self) -> AppResult<serde_json::Value> {
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| AppError::XbxEngine("Failed to lock xbxengine runtime".to_string()))?;
        let viewport_id = runtime
            .snapshot()
            .viewport
            .as_ref()
            .map(|viewport| viewport.viewport_id.clone());
        self.sync_native_video_host_timing(&mut runtime, viewport_id.as_deref());
        let mut stats = runtime.snapshot_stats();
        self.apply_native_video_host_stats(&mut stats, viewport_id.as_deref());
        Ok(serde_json::to_value(stats)?)
    }

    fn record_runtime_trace_observations(
        &self,
        session_id: Option<&str>,
        stats: &XbxEngineStatsDto,
    ) {
        let Ok(mut observation_state) = self.last_trace_observation.lock() else {
            return;
        };
        record_runtime_trace_observations(
            &self.runtime_trace,
            &mut observation_state,
            session_id,
            stats,
        );
    }

    fn apply_native_video_host_stats(
        &self,
        stats: &mut XbxEngineStatsDto,
        viewport_id: Option<&str>,
    ) {
        let Some(viewport_id) = viewport_id else {
            return;
        };
        let Some(viewport) = self.native_video_snapshot(viewport_id) else {
            return;
        };

        stats.present_fps = Some(viewport.host_present_fps);
        stats.video_present_submit_count_total = Some(viewport.host_present_submit_count_total);
        stats.video_present_drop_count_total = Some(viewport.host_present_drop_count_total);
        stats.video_present_overwrite_count_total =
            Some(viewport.host_present_overwrite_count_total);
        stats.video_present_descriptor_upload_mode = viewport.host_descriptor_upload_mode.clone();
        stats.video_present_descriptor_metal_import_count_total =
            Some(viewport.host_descriptor_metal_import_count_total);
        stats.video_present_descriptor_cpu_upload_count_total =
            Some(viewport.host_descriptor_cpu_upload_count_total);

        if let Some(latest_present_time_ms) = viewport.latest_host_present_time_ms {
            let host_now_ms = current_time_ms_f64();
            stats.present_age_ms = Some((host_now_ms - latest_present_time_ms).max(0.0));
        }
    }

    fn native_video_snapshot(&self, viewport_id: &str) -> Option<NativeVideoViewportState> {
        let Ok(registry) = self.native_video.lock() else {
            return None;
        };
        registry.snapshot(viewport_id)
    }

    fn sync_native_video_host_timing(
        &self,
        runtime: &mut TauriXbxEngineRuntime,
        viewport_id: Option<&str>,
    ) {
        let Some(viewport_id) = viewport_id else {
            return;
        };
        let Some(viewport) = self.native_video_snapshot(viewport_id) else {
            return;
        };
        let _ = runtime.update_host_video_timing(
            viewport.host_display_interval_ms,
            viewport.host_frame_age_budget_ms,
        );
    }
}

fn current_time_ms_f64() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}

// 为 runtime trace 生成一份稳定的 SDP 能力摘要，便于直接判断远端是否宣告 repair/fec 能力。
fn summarize_sdp_capabilities(sdp: &str) -> serde_json::Value {
    let mut video_payload_order = Vec::new();
    let mut payload_codec_map = BTreeMap::<String, String>::new();
    let mut apt_pairs = Vec::<serde_json::Value>::new();
    let mut has_video_fid_group = false;

    for line in sdp.split("\r\n") {
        if let Some(rest) = line.strip_prefix("m=video ") {
            let parts = rest.split_whitespace().collect::<Vec<_>>();
            // m=video 格式是: <port> <proto> <payload...>
            if parts.len() > 2 {
                video_payload_order = parts[2..]
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect();
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix("a=rtpmap:") {
            let mut payload_and_codec = rest.split_whitespace();
            let Some(payload_type) = payload_and_codec.next() else {
                continue;
            };
            let Some(codec_part) = payload_and_codec.next() else {
                continue;
            };
            let codec = codec_part
                .split('/')
                .next()
                .unwrap_or(codec_part)
                .to_ascii_lowercase();
            payload_codec_map.insert(payload_type.to_string(), codec);
            continue;
        }

        if let Some(rest) = line.strip_prefix("a=fmtp:") {
            let mut payload_and_params = rest.splitn(2, ' ');
            let Some(payload_type) = payload_and_params.next() else {
                continue;
            };
            let Some(params) = payload_and_params.next() else {
                continue;
            };
            for param in params.split(';') {
                let trimmed = param.trim();
                let Some(apt_value) = trimmed.strip_prefix("apt=") else {
                    continue;
                };
                apt_pairs.push(serde_json::json!({
                    "payloadType": payload_type,
                    "aptPayloadType": apt_value,
                    "codec": payload_codec_map.get(payload_type).cloned(),
                }));
            }
            continue;
        }

        if line.starts_with("a=ssrc-group:FID ") {
            has_video_fid_group = true;
        }
    }

    let mut declared_video_codecs = BTreeSet::new();
    let mut declared_video_repair_codecs = BTreeSet::new();
    for payload_type in &video_payload_order {
        let Some(codec) = payload_codec_map.get(payload_type) else {
            continue;
        };
        match codec.as_str() {
            "rtx" | "red" | "ulpfec" | "flexfec-03" | "flexfec" => {
                declared_video_repair_codecs.insert(codec.clone());
            }
            _ => {
                declared_video_codecs.insert(codec.clone());
            }
        }
    }

    serde_json::json!({
        "hasVideo": !video_payload_order.is_empty(),
        "videoPayloadOrder": video_payload_order,
        "videoCodecs": declared_video_codecs.into_iter().collect::<Vec<_>>(),
        "videoRepairCodecs": declared_video_repair_codecs.into_iter().collect::<Vec<_>>(),
        "hasVideoRtx": payload_codec_map.values().any(|codec| codec == "rtx"),
        "hasVideoRed": payload_codec_map.values().any(|codec| codec == "red"),
        "hasVideoUlpfec": payload_codec_map.values().any(|codec| codec == "ulpfec"),
        "hasVideoFlexfec": payload_codec_map
            .values()
            .any(|codec| codec == "flexfec-03" || codec == "flexfec"),
        "hasSsrcGroupFid": has_video_fid_group,
        "aptPairs": apt_pairs,
    })
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
        self.runtime_trace.record_event(
            "xbxengine-host",
            "exchangeOfferRequested",
            Some(&session_id),
            serde_json::json!({
                "channel": channel,
                "restart": restart,
                "offerSdp": sdp,
                "offerSdpCapabilities": summarize_sdp_capabilities(&sdp),
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
        self.runtime_trace.record_event(
            "xbxengine-host",
            "exchangeOfferResult",
            None,
            serde_json::json!({
                "answerSdp": result.answer.sdp,
                "answerSdpCapabilities": summarize_sdp_capabilities(&result.answer.sdp),
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
        self.runtime_trace.record_event(
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
        self.runtime_trace.record_event(
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
        self.runtime_trace.record_event(
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
        self.runtime_trace.record_event(
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
        self.runtime_trace.record_event(
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
        self.runtime_trace.record_event(
            "xbxengine-host",
            "closeRemoteSessionResult",
            None,
            serde_json::json!({ "closed": true }),
        );
        Ok(XbxEngineHostResponseDto::RemoteSessionClosed)
    }

    fn attach_native_viewport(
        &self,
        viewport: &xbxengine_protocol::XbxEngineViewportDto,
        surface_id: Option<&str>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let Ok(mut registry) = self.native_video.lock() else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineNativeVideoRegistryLockFailed",
            ));
        };
        let changed = registry.attach_viewport(&viewport.viewport_id, surface_id);
        if changed {
            self.runtime_trace.record_state(
                "xbxengine-host",
                "nativeViewportAttached",
                None,
                serde_json::json!({
                    "viewportId": viewport.viewport_id,
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
        self.runtime_trace.record_state(
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

    fn log_gamepad_rumble_result(
        &self,
        event_name: &'static str,
        request_summary: serde_json::Value,
        result: &OhMyGamepadRumbleResultDto,
    ) {
        self.runtime_trace.record_event(
            "xbxengine-host",
            event_name,
            None,
            serde_json::json!({
                "request": request_summary,
                "accepted": result.accepted,
                "reason": result.reason,
                "resolvedDeviceIds": result.resolved_device_ids,
            }),
        );
        if !result.accepted {
            log::warn!(
                "[xbxengine][host] gamepad rumble rejected reason={:?} resolved_device_ids={:?}",
                result.reason,
                result.resolved_device_ids
            );
        }
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
        self.attach_native_viewport(viewport, surface_id)
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

    fn play_gamepad_rumble(
        &mut self,
        request: OhMyGamepadRumbleRequestDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        let state = self
            .app_state()
            .map_err(map_app_error("playGamepadRumble"))?;
        let vibration_config = state.config.get_streaming_config();
        let request_summary = serde_json::to_value(&request).unwrap_or(serde_json::Value::Null);
        if !vibration_config.vibration {
            let result = OhMyGamepadRumbleResultDto {
                accepted: false,
                reason: Some(
                    ohmygamepad_protocol::OhMyGamepadRumbleRejectionReasonDto::Unsupported,
                ),
                resolved_device_ids: Vec::new(),
            };
            self.log_gamepad_rumble_result("playGamepadRumbleResult", request_summary, &result);
            return Ok(());
        }
        let result = state
            .gamepad
            .play_rumble(request)
            .map_err(|error| map_app_error("playGamepadRumble")(AppError::Gamepad(error)))?;
        self.log_gamepad_rumble_result("playGamepadRumbleResult", request_summary, &result);
        Ok(())
    }

    fn stop_gamepad_rumble(
        &mut self,
        target: OhMyGamepadRumbleTargetDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        let state = self
            .app_state()
            .map_err(map_app_error("stopGamepadRumble"))?;
        let vibration_config = state.config.get_streaming_config();
        let request_summary = serde_json::json!({ "target": &target });
        if !vibration_config.vibration {
            let result = OhMyGamepadRumbleResultDto {
                accepted: false,
                reason: Some(
                    ohmygamepad_protocol::OhMyGamepadRumbleRejectionReasonDto::Unsupported,
                ),
                resolved_device_ids: Vec::new(),
            };
            self.log_gamepad_rumble_result("stopGamepadRumbleResult", request_summary, &result);
            return Ok(());
        }
        let result = state
            .gamepad
            .stop_rumble(target)
            .map_err(|error| map_app_error("stopGamepadRumble")(AppError::Gamepad(error)))?;
        self.log_gamepad_rumble_result("stopGamepadRumbleResult", request_summary, &result);
        Ok(())
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
                self.runtime_trace.record_event(
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

#[cfg(test)]
mod tests {
    use super::summarize_sdp_capabilities;

    #[test]
    fn summarize_sdp_capabilities_detects_video_repair_streams() {
        let sdp = concat!(
            "v=0\r\n",
            "m=video 9 UDP/TLS/RTP/SAVPF 102 121 116\r\n",
            "a=rtpmap:102 H264/90000\r\n",
            "a=rtpmap:121 rtx/90000\r\n",
            "a=fmtp:121 apt=102\r\n",
            "a=rtpmap:116 ulpfec/90000\r\n",
            "a=ssrc-group:FID 1111 2222\r\n",
        );

        let summary = summarize_sdp_capabilities(sdp);

        assert_eq!(summary["hasVideoRtx"], true);
        assert_eq!(summary["hasVideoUlpfec"], true);
        assert_eq!(summary["hasSsrcGroupFid"], true);
        assert_eq!(summary["videoCodecs"], serde_json::json!(["h264"]));
        assert_eq!(
            summary["videoRepairCodecs"],
            serde_json::json!(["rtx", "ulpfec"])
        );
        assert_eq!(
            summary["aptPairs"],
            serde_json::json!([{
                "payloadType": "121",
                "aptPayloadType": "102",
                "codec": "rtx"
            }])
        );
    }

    #[test]
    fn summarize_sdp_capabilities_handles_plain_h264_offer() {
        let sdp = concat!(
            "v=0\r\n",
            "m=video 9 UDP/TLS/RTP/SAVPF 102 104\r\n",
            "a=rtpmap:102 H264/90000\r\n",
            "a=fmtp:102 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42001f\r\n",
            "a=rtpmap:104 H264/90000\r\n",
            "a=fmtp:104 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n",
        );

        let summary = summarize_sdp_capabilities(sdp);

        assert_eq!(summary["hasVideo"], true);
        assert_eq!(summary["hasVideoRtx"], false);
        assert_eq!(summary["hasVideoUlpfec"], false);
        assert_eq!(summary["hasVideoRed"], false);
        assert_eq!(summary["hasVideoFlexfec"], false);
        assert_eq!(summary["hasSsrcGroupFid"], false);
        assert_eq!(summary["videoRepairCodecs"], serde_json::json!([]));
        assert_eq!(summary["videoCodecs"], serde_json::json!(["h264"]));
    }
}
