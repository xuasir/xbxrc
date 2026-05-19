use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use ohmygamepad_protocol::OhMyGamepadRumbleRequestDto;
use tauri::{AppHandle, Manager};
use xbxengine::{
    create_active_media_backend, OhMyGamepadXbxEngineInputBackend, XbxEngineEventSink,
    XbxEngineHostBridge, XbxEngineHostPresentRoute, XbxEngineHostVideoPresentMetrics,
    XbxEngineMediaBackend, XbxEngineRenderFrame, XbxEngineRuntime, XbxEngineRuntimeConfig,
    XbxEngineRuntimeError, XbxHostRenderFramePush, XbxHostRenderFramePushOutcome,
};
use xbxengine_protocol::XbxEngineStatsDto;
use xbxengine_protocol::{
    XbxEngineControlCommandDto, XbxEngineHostRequestDto, XbxEngineHostResponseDto,
    XbxEngineIceCandidateDto, XbxEngineRuntimeEventDto,
};

use crate::error::{AppError, AppResult};
use crate::mods::native_video::{NativeVideoDisplayState, NativeVideoRegistryRef};
use crate::mods::runtime_trace::RuntimeTraceRecorderRef;
use crate::mods::streaming::{
    StreamingCloseSessionParams, StreamingExchangeOfferParams, StreamingPollIceParams,
    StreamingSubmitIceParams,
};
use crate::mods::xbxengine::build_info::current_build_fingerprint_with_effective;
use crate::mods::xbxengine::rumble_worker::GamepadRumbleWorkerHandle;
use crate::mods::xbxengine::trace_projection::{
    build_observability_snapshot, record_runtime_trace_observations, should_skip_trace_tick,
    RuntimeTraceObservationState,
};
use crate::shell::bridge::{TauriEngineEventBridge, TauriEngineWindowHost};
use crate::AppState;

/// 推式 host present：`renderer` 线程调用；`route` 由 engine runtime 快照同步。
struct NativeVideoHostRenderFramePush {
    native_video: NativeVideoRegistryRef,
    route: Arc<StdMutex<XbxEngineHostPresentRoute>>,
    runtime_trace: RuntimeTraceRecorderRef,
}

impl XbxHostRenderFramePush for NativeVideoHostRenderFramePush {
    fn push_render_frame_for_host_present(
        &self,
        frame: XbxEngineRenderFrame,
    ) -> XbxHostRenderFramePushOutcome {
        let Ok(route_guard) = self.route.lock() else {
            self.runtime_trace.record_event(
                "xbxengine-host",
                "nativeHostPushDropped",
                None,
                serde_json::json!({
                    "reason": "routeUnavailable",
                    "frameSeq": frame.frame_seq,
                }),
            );
            return XbxHostRenderFramePushOutcome::RouteUnavailable;
        };
        let Some(viewport) = route_guard.viewport.as_ref() else {
            self.runtime_trace.record_event(
                "xbxengine-host",
                "nativeHostPushDropped",
                None,
                serde_json::json!({
                    "reason": "routeUnavailable",
                    "frameSeq": frame.frame_seq,
                }),
            );
            return XbxHostRenderFramePushOutcome::RouteUnavailable;
        };
        let Ok(mut registry) = self.native_video.lock() else {
            self.runtime_trace.record_event(
                "xbxengine-host",
                "nativeHostPushDropped",
                None,
                serde_json::json!({
                    "reason": "registryUnavailable",
                    "frameSeq": frame.frame_seq,
                    "viewportId": viewport.viewport_id,
                    "surfaceId": route_guard.surface_id,
                }),
            );
            return XbxHostRenderFramePushOutcome::RegistryUnavailable;
        };
        if registry.present_frame(
            &viewport.viewport_id,
            route_guard.surface_id.as_deref(),
            &frame,
        ) {
            XbxHostRenderFramePushOutcome::Accepted
        } else {
            XbxHostRenderFramePushOutcome::Rejected
        }
    }
}

type TauriXbxEngineRuntime = XbxEngineRuntime<
    TauriXbxEngineHostBridge,
    TauriXbxEngineEventSink,
    Box<dyn XbxEngineMediaBackend>,
>;

#[derive(Debug)]
struct NativeVideoHostFeedbackSnapshot {
    viewport_id: String,
    host_display_interval_ms: Option<f64>,
    host_frame_age_budget_ms: Option<f64>,
    present_metrics: XbxEngineHostVideoPresentMetrics,
    pending_frame_drops: Vec<xbxengine::XbxEngineHostVideoFrameDropEvent>,
}

/// 运行态只负责持有 runtime 实例和并发访问。
pub struct XbxEngineRuntimeState {
    runtime: StdMutex<TauriXbxEngineRuntime>,
    native_video: NativeVideoRegistryRef,
    runtime_trace: RuntimeTraceRecorderRef,
    /// 与 `XbxEngineRuntime` 快照同步，供 `NativeVideoHostRenderFramePush` 在 renderer 线程读取。
    host_present_route: Arc<StdMutex<XbxEngineHostPresentRoute>>,
    /// `statsSnapshot` / `observabilitySnapshot` 最小间隔（毫秒），可由设置 `runtime_trace_mode` 运行时更新。
    stats_snapshot_interval_ms: Arc<AtomicU64>,
    last_stats_trace_at: StdMutex<Option<Instant>>,
    last_trace_observation: StdMutex<RuntimeTraceObservationState>,
    active_session_id: StdMutex<Option<String>>,
    cancellation_epoch: Arc<AtomicU64>,
    rumble_worker: GamepadRumbleWorkerHandle,
}

impl XbxEngineRuntimeState {
    pub fn new(
        app_handle: AppHandle,
        last_runtime_event: Arc<StdMutex<Option<serde_json::Value>>>,
        native_video: NativeVideoRegistryRef,
        runtime_trace: RuntimeTraceRecorderRef,
        stats_snapshot_interval: Duration,
    ) -> Self {
        let cancellation_epoch = Arc::new(AtomicU64::new(0));
        let event_bridge = TauriEngineEventBridge {
            app_handle: app_handle.clone(),
            state: Default::default(),
            last_runtime_event,
            runtime_trace: runtime_trace.clone(),
        };
        let input_backend = Box::new(OhMyGamepadXbxEngineInputBackend::new());
        let host_present_route = Arc::new(StdMutex::new(XbxEngineHostPresentRoute::default()));
        let host_render_frame_push: Arc<dyn XbxHostRenderFramePush> =
            Arc::new(NativeVideoHostRenderFramePush {
                native_video: native_video.clone(),
                route: host_present_route.clone(),
                runtime_trace: runtime_trace.clone(),
            });
        let media_backend = create_active_media_backend(
            input_backend,
            XbxEngineRuntimeConfig::default(),
            Some(host_render_frame_push),
        );
        let rumble_worker =
            GamepadRumbleWorkerHandle::new(app_handle.clone(), runtime_trace.clone());
        let runtime = XbxEngineRuntime::with_media_backend(
            XbxEngineRuntimeConfig::default(),
            TauriXbxEngineHostBridge {
                app_handle,
                native_video: native_video.clone(),
                runtime_trace: runtime_trace.clone(),
                host_present_route: host_present_route.clone(),
                cancellation_epoch: cancellation_epoch.clone(),
                rumble_worker: rumble_worker.clone(),
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
            host_present_route,
            stats_snapshot_interval_ms: Arc::new(AtomicU64::new(
                stats_snapshot_interval.as_millis() as u64,
            )),
            last_stats_trace_at: StdMutex::new(None),
            last_trace_observation: StdMutex::new(RuntimeTraceObservationState::default()),
            active_session_id: StdMutex::new(None),
            cancellation_epoch,
            rumble_worker,
        }
    }

    pub fn set_stats_snapshot_interval(&self, interval: Duration) {
        self.stats_snapshot_interval_ms
            .store(interval.as_millis() as u64, Ordering::Relaxed);
    }

    fn sync_host_present_route_from_runtime(&self, runtime: &TauriXbxEngineRuntime) {
        let snap = runtime.snapshot();
        if let Ok(mut guard) = self.host_present_route.lock() {
            guard.viewport = snap.viewport.clone();
            guard.surface_id = snap.surface_id.clone();
        }
    }

    pub fn apply_control(
        &self,
        command: XbxEngineControlCommandDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        let command_value = serde_json::to_value(&command).unwrap_or(serde_json::Value::Null);
        let session_id = extract_command_session_id(&command);
        record_control_apply_trace(&self.runtime_trace, &command, "entered", None, None);
        self.runtime_trace.record_event(
            "xbxengine",
            "controlCommand",
            session_id.as_deref(),
            command_value,
        );
        match &command {
            // Stop 需要先发出取消信号，打断正在路上的 keepalive/offer/ice。
            XbxEngineControlCommandDto::StopRuntime { reason } => {
                self.cancellation_epoch.fetch_add(1, Ordering::SeqCst);
                log::warn!(
                    "[xbxengine][control] stop runtime requested reason={}",
                    reason.as_deref().unwrap_or("unspecified")
                );
            }
            XbxEngineControlCommandDto::StartRuntime { .. } => {}
            _ => {}
        }
        if let Ok(mut active_session_id) = self.active_session_id.lock() {
            match &command {
                XbxEngineControlCommandDto::StartRuntime { session, .. } => {
                    *active_session_id = Some(session.session_id.clone());
                }
                XbxEngineControlCommandDto::StopRuntime { .. } => {
                    *active_session_id = None;
                }
                _ => {}
            }
        }
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRuntimeLockPoisoned"))?;
        record_control_apply_trace(&self.runtime_trace, &command, "lockAcquired", None, None);
        let started_at = Instant::now();
        let command_for_trace = command.clone();
        let result = runtime.apply_control(command);
        self.sync_host_present_route_from_runtime(&runtime);
        record_control_apply_trace(
            &self.runtime_trace,
            &command_for_trace,
            "completed",
            Some(started_at.elapsed().as_millis()),
            result.as_ref().err(),
        );
        result
    }

    pub fn tick(&self) -> Result<(), XbxEngineRuntimeError> {
        let viewport_id = self.current_viewport_id()?;
        let native_video_feedback = self.collect_native_video_host_feedback(viewport_id.as_deref());
        let stats_snapshot = {
            let mut runtime = self
                .runtime
                .lock()
                .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRuntimeLockPoisoned"))?;
            self.sync_host_present_route_from_runtime(&runtime);
            self.apply_native_video_host_feedback(&mut runtime, native_video_feedback);
            runtime.tick();
            let mut stats_snapshot = runtime.snapshot_stats();
            self.apply_build_fingerprint(&mut stats_snapshot);
            stats_snapshot
        };
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
        let interval =
            Duration::from_millis(self.stats_snapshot_interval_ms.load(Ordering::Relaxed));
        let should_record = last_stats_trace_at
            .map(|last| now.duration_since(last) >= interval)
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
        let viewport_id = self
            .current_viewport_id()
            .map_err(|_| AppError::XbxEngine("Failed to lock xbxengine runtime".to_string()))?;
        let native_video_feedback = self.collect_native_video_host_feedback(viewport_id.as_deref());
        let stats = {
            let mut runtime = self
                .runtime
                .lock()
                .map_err(|_| AppError::XbxEngine("Failed to lock xbxengine runtime".to_string()))?;
            self.sync_host_present_route_from_runtime(&runtime);
            self.apply_native_video_host_feedback(&mut runtime, native_video_feedback);
            let mut stats = runtime.snapshot_stats();
            self.apply_build_fingerprint(&mut stats);
            stats
        };
        Ok(serde_json::to_value(stats)?)
    }

    pub fn shutdown(&self) {
        if let Err(error) = self.rumble_worker.shutdown() {
            log::warn!("xbxengine rumble worker shutdown failed: {}", error);
        }
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

    fn apply_build_fingerprint(&self, stats: &mut XbxEngineStatsDto) {
        let effective_feedback_interval_ms = stats
            .latest_rtc_builder_observation
            .as_ref()
            .map(|observation| observation.feedback_interval_ms.max(0.0).round() as u64)
            .unwrap_or_else(|| {
                xbxengine::XbxEngineRuntimeConfig::default()
                    .webrtc
                    .video_pipeline
                    .feedback_interval_ms
            });
        stats.build_fingerprint = Some(current_build_fingerprint_with_effective(
            effective_feedback_interval_ms,
        ));
    }

    fn current_viewport_id(&self) -> Result<Option<String>, XbxEngineRuntimeError> {
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRuntimeLockPoisoned"))?;
        Ok(runtime
            .snapshot()
            .viewport
            .as_ref()
            .map(|viewport| viewport.viewport_id.clone()))
    }

    fn collect_native_video_host_feedback(
        &self,
        viewport_id: Option<&str>,
    ) -> Option<NativeVideoHostFeedbackSnapshot> {
        let Some(viewport_id) = viewport_id else {
            return None;
        };
        let Ok(mut registry) = self.native_video.lock() else {
            return None;
        };
        let Some(viewport) = registry.snapshot(viewport_id) else {
            return None;
        };
        Some(NativeVideoHostFeedbackSnapshot {
            viewport_id: viewport_id.to_string(),
            host_display_interval_ms: viewport.host_display_interval_ms,
            host_frame_age_budget_ms: viewport.host_frame_age_budget_ms,
            present_metrics: XbxEngineHostVideoPresentMetrics {
                latest_host_submit_time_ms: viewport.latest_host_submit_time_ms,
                latest_host_submit_rtp_timestamp: viewport.latest_host_submit_rtp_timestamp,
                // 使用 native_video telemetry 的真实 present 时间，统一 runtime/owner/snapshot 语义。
                latest_host_present_time_ms: viewport.latest_host_present_time_ms,
                host_view_generation: viewport.host_view_generation,
                latest_host_view_created_at_ms: viewport.latest_host_view_created_at_ms,
                host_mailbox_submit_epoch: viewport.host_mailbox_submit_epoch,
                host_display_tick_epoch: viewport.host_display_tick_epoch,
                host_frame_present_epoch: viewport.host_frame_present_epoch,
                cadence_phase: viewport.host_cadence_phase.clone(),
                last_displayed_frame_seq: viewport.last_displayed_frame_seq,
                last_displayed_frame_rtp_timestamp: viewport.last_displayed_frame_rtp_timestamp,
                last_displayed_at_ms: viewport.last_displayed_at_ms,
                present_fps: viewport.host_present_fps,
                // core 里该字段历史命名为 submit，本质是宿主侧 enqueue 次数。
                host_mailbox_enqueue_count_total: viewport.host_mailbox_enqueue_count_total,
                host_mailbox_drop_count_total: viewport.host_mailbox_drop_count_total,
                host_mailbox_overwrite_count_total: viewport.host_mailbox_overwrite_count_total,
                no_pending_take_count_total: viewport.host_no_pending_take_count_total,
                no_pending_streak: viewport.host_no_pending_streak,
                no_pending_max_streak: viewport.host_no_pending_max_streak,
                descriptor_upload_mode: viewport.host_descriptor_upload_mode.clone(),
                descriptor_metal_import_count_total: viewport
                    .host_descriptor_metal_import_count_total,
                descriptor_cpu_upload_count_total: viewport.host_descriptor_cpu_upload_count_total,
            },
            pending_frame_drops: registry.take_pending_host_frame_drops(viewport_id),
        })
    }

    fn apply_native_video_host_feedback(
        &self,
        runtime: &mut TauriXbxEngineRuntime,
        feedback: Option<NativeVideoHostFeedbackSnapshot>,
    ) {
        let Some(feedback) = feedback else {
            return;
        };
        let runtime_viewport_id = runtime
            .snapshot()
            .viewport
            .as_ref()
            .map(|viewport| viewport.viewport_id.as_str());
        if runtime_viewport_id != Some(feedback.viewport_id.as_str()) {
            return;
        }
        let _ = runtime.update_host_video_timing(
            feedback.host_display_interval_ms,
            feedback.host_frame_age_budget_ms,
        );
        let _ = runtime.update_host_video_present_metrics(feedback.present_metrics);
        for drop in feedback.pending_frame_drops {
            let _ = runtime.record_host_video_frame_drop(drop);
        }
    }
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

fn update_host_present_route_on_attach(
    route: &Arc<StdMutex<XbxEngineHostPresentRoute>>,
    viewport: &xbxengine_protocol::XbxEngineViewportDto,
    surface_id: Option<&str>,
) {
    if let Ok(mut guard) = route.lock() {
        guard.viewport = Some(viewport.clone());
        guard.surface_id = surface_id.map(str::to_string);
    }
}

fn update_host_present_route_on_detach(
    route: &Arc<StdMutex<XbxEngineHostPresentRoute>>,
    viewport_id: &str,
) {
    if let Ok(mut guard) = route.lock() {
        if guard
            .viewport
            .as_ref()
            .is_some_and(|viewport| viewport.viewport_id == viewport_id)
        {
            guard.viewport = None;
            guard.surface_id = None;
        }
    }
}

#[derive(Clone)]
struct TauriXbxEngineHostBridge {
    app_handle: AppHandle,
    native_video: NativeVideoRegistryRef,
    runtime_trace: RuntimeTraceRecorderRef,
    host_present_route: Arc<StdMutex<XbxEngineHostPresentRoute>>,
    cancellation_epoch: Arc<AtomicU64>,
    rumble_worker: GamepadRumbleWorkerHandle,
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
        update_host_present_route_on_attach(&self.host_present_route, viewport, surface_id);
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
        update_host_present_route_on_detach(&self.host_present_route, viewport_id);
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

    fn apply_native_display_state(
        &self,
        viewport_id: &str,
        state: &xbxengine_protocol::XbxEngineDisplayStateDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        let Ok(mut registry) = self.native_video.lock() else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineNativeVideoRegistryLockFailed",
            ));
        };
        registry.apply_display_state(
            viewport_id,
            NativeVideoDisplayState::from_video_format(state.video_format.as_deref()),
        );
        self.runtime_trace.record_state(
            "xbxengine-host",
            "nativeDisplayStateApplied",
            None,
            serde_json::json!({
                "viewportId": viewport_id,
                "videoFormat": state.video_format.clone(),
            }),
        );
        Ok(())
    }

    fn reset_native_presenter_for_host_stall(
        &self,
        viewport_id: &str,
    ) -> Result<(), XbxEngineRuntimeError> {
        let Ok(mut registry) = self.native_video.lock() else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineNativeVideoRegistryLockFailed",
            ));
        };
        registry.reset_presenter_for_host_stall_recovery(viewport_id);
        self.runtime_trace.record_state(
            "xbxengine-host",
            "nativePresenterResetHostStall",
            None,
            serde_json::json!({ "viewportId": viewport_id }),
        );
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

    fn apply_display_state(
        &mut self,
        viewport: Option<&xbxengine_protocol::XbxEngineViewportDto>,
        state: &xbxengine_protocol::XbxEngineDisplayStateDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        let Some(viewport) = viewport else {
            return Ok(());
        };
        self.apply_native_display_state(&viewport.viewport_id, state)
    }

    fn reset_native_video_presenter_for_host_stall(
        &mut self,
        viewport_id: &str,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.reset_native_presenter_for_host_stall(viewport_id)
    }

    fn reset_native_video_presenter_for_display_recovery(
        &mut self,
        viewport_id: &str,
        reason: &str,
    ) -> Result<(), XbxEngineRuntimeError> {
        let Ok(mut registry) = self.native_video.lock() else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineNativeVideoRegistryLockFailed",
            ));
        };
        registry.reset_presenter_for_display_recovery(viewport_id, reason);
        self.runtime_trace.record_state(
            "xbxengine-host",
            "nativePresenterResetDisplayRecovery",
            None,
            serde_json::json!({ "viewportId": viewport_id, "reason": reason }),
        );
        Ok(())
    }

    fn submit_gamepad_rumble_request(
        &mut self,
        request: OhMyGamepadRumbleRequestDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.rumble_worker.submit_request(request)
    }

    fn clear_pending_gamepad_rumble_requests(&mut self) -> Result<(), XbxEngineRuntimeError> {
        self.rumble_worker.clear_pending_requests()
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

pub(super) fn map_app_error(
    action: &'static str,
) -> impl FnOnce(AppError) -> XbxEngineRuntimeError {
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

fn extract_command_viewport_id(command: &XbxEngineControlCommandDto) -> Option<String> {
    match command {
        XbxEngineControlCommandDto::StartRuntime { viewport, .. }
        | XbxEngineControlCommandDto::AttachViewport { viewport } => {
            Some(viewport.viewport_id.clone())
        }
        _ => None,
    }
}

fn extract_command_target_type(command: &XbxEngineControlCommandDto) -> Option<&'static str> {
    match command {
        XbxEngineControlCommandDto::StartRuntime { session, .. } => {
            Some(match session.target_type {
                xbxengine_protocol::XbxEngineTargetTypeDto::Home => "home",
                xbxengine_protocol::XbxEngineTargetTypeDto::Cloud => "cloud",
            })
        }
        _ => None,
    }
}

fn control_apply_event_name(
    command: &XbxEngineControlCommandDto,
    stage: &'static str,
) -> Option<&'static str> {
    match (command, stage) {
        (XbxEngineControlCommandDto::AttachViewport { .. }, "entered") => {
            Some("runtimeAttachViewportApplyEntered")
        }
        (XbxEngineControlCommandDto::AttachViewport { .. }, "lockAcquired") => {
            Some("runtimeAttachViewportRuntimeLockAcquired")
        }
        (XbxEngineControlCommandDto::AttachViewport { .. }, "completed") => {
            Some("runtimeAttachViewportApplyCompleted")
        }
        (XbxEngineControlCommandDto::StartRuntime { .. }, "entered") => {
            Some("runtimeStartApplyEntered")
        }
        (XbxEngineControlCommandDto::StartRuntime { .. }, "lockAcquired") => {
            Some("runtimeStartRuntimeLockAcquired")
        }
        (XbxEngineControlCommandDto::StartRuntime { .. }, "completed") => {
            Some("runtimeStartApplyCompleted")
        }
        _ => None,
    }
}

fn record_control_apply_trace(
    runtime_trace: &RuntimeTraceRecorderRef,
    command: &XbxEngineControlCommandDto,
    stage: &'static str,
    duration_ms: Option<u128>,
    error: Option<&XbxEngineRuntimeError>,
) {
    let Some(event_name) = control_apply_event_name(command, stage) else {
        return;
    };
    let session_id = extract_command_session_id(command);
    let viewport_id = extract_command_viewport_id(command);
    let target_type = extract_command_target_type(command);
    runtime_trace.record_event(
        "xbxengine-host",
        event_name,
        session_id.as_deref(),
        serde_json::json!({
            "sessionId": session_id,
            "viewportId": viewport_id,
            "targetType": target_type,
            "durationMs": duration_ms,
            "ok": error.is_none(),
            "error": error.map(ToString::to_string),
        }),
    );
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex as StdMutex};

    use super::{
        summarize_sdp_capabilities, update_host_present_route_on_attach,
        update_host_present_route_on_detach,
    };
    use xbxengine::XbxEngineHostPresentRoute;
    use xbxengine_protocol::XbxEngineViewportDto;

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

    #[test]
    fn attach_updates_host_present_route_immediately() {
        let route = Arc::new(StdMutex::new(XbxEngineHostPresentRoute::default()));
        let viewport = XbxEngineViewportDto {
            viewport_id: "vp-1".to_string(),
        };

        update_host_present_route_on_attach(&route, &viewport, Some("wgpu:surface-1"));

        let guard = route.lock().expect("route lock");
        assert_eq!(
            guard
                .viewport
                .as_ref()
                .map(|value| value.viewport_id.as_str()),
            Some("vp-1")
        );
        assert_eq!(guard.surface_id.as_deref(), Some("wgpu:surface-1"));
    }

    #[test]
    fn detach_clears_matching_host_present_route() {
        let route = Arc::new(StdMutex::new(XbxEngineHostPresentRoute {
            viewport: Some(XbxEngineViewportDto {
                viewport_id: "vp-1".to_string(),
            }),
            surface_id: Some("wgpu:surface-1".to_string()),
        }));

        update_host_present_route_on_detach(&route, "vp-1");

        let guard = route.lock().expect("route lock");
        assert!(guard.viewport.is_none());
        assert!(guard.surface_id.is_none());
    }
}
