use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, TryLockError};
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
use crate::mods::runtime_trace::{trace_observation_tick_interval, RuntimeTraceRecorderRef};
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

const XBXENGINE_HOST_EXCHANGE_OFFER_TIMEOUT: Duration = Duration::from_secs(20);
static XBXENGINE_HOST_EXCHANGE_OFFER_REQUEST_SEQ: AtomicU64 = AtomicU64::new(1);
const XBXENGINE_RUNTIME_LOCK_TRACE_INTERVAL: Duration = Duration::from_millis(1_000);
const XBXENGINE_INITIAL_RUNTIME_GENERATION: u64 = 1;

struct PendingControlGuard<'a>(&'a AtomicU64);

impl Drop for PendingControlGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

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
        let (viewport_id, surface_id) = match self.route.try_lock() {
            Ok(route_guard) => {
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
                (viewport.viewport_id.clone(), route_guard.surface_id.clone())
            }
            Err(TryLockError::WouldBlock) => {
                self.runtime_trace.record_event(
                    "xbxengine-host",
                    "nativeHostPushDropped",
                    None,
                    serde_json::json!({
                        "reason": "routeBusy",
                        "frameSeq": frame.frame_seq,
                    }),
                );
                return XbxHostRenderFramePushOutcome::RouteUnavailable;
            }
            Err(TryLockError::Poisoned(_)) => {
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
            }
        };
        let mut registry = match self.native_video.try_lock() {
            Ok(registry) => registry,
            Err(TryLockError::WouldBlock) => {
                self.runtime_trace.record_event(
                    "xbxengine-host",
                    "nativeHostPushDropped",
                    None,
                    serde_json::json!({
                        "reason": "registryBusy",
                        "frameSeq": frame.frame_seq,
                        "viewportId": viewport_id.as_str(),
                        "surfaceId": surface_id.as_deref(),
                    }),
                );
                return XbxHostRenderFramePushOutcome::RegistryUnavailable;
            }
            Err(TryLockError::Poisoned(_)) => {
                self.runtime_trace.record_event(
                    "xbxengine-host",
                    "nativeHostPushDropped",
                    None,
                    serde_json::json!({
                        "reason": "registryUnavailable",
                        "frameSeq": frame.frame_seq,
                        "viewportId": viewport_id.as_str(),
                        "surfaceId": surface_id.as_deref(),
                    }),
                );
                return XbxHostRenderFramePushOutcome::RegistryUnavailable;
            }
        };
        if registry.present_frame(&viewport_id, surface_id.as_deref(), &frame) {
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

#[allow(clippy::too_many_arguments)]
fn create_tauri_runtime(
    app_handle: AppHandle,
    last_runtime_event: Arc<StdMutex<Option<serde_json::Value>>>,
    native_video: NativeVideoRegistryRef,
    runtime_trace: RuntimeTraceRecorderRef,
    host_present_route: Arc<StdMutex<XbxEngineHostPresentRoute>>,
    cancellation_epoch: Arc<AtomicU64>,
    rumble_worker: GamepadRumbleWorkerHandle,
    active_event_generation: Arc<AtomicU64>,
    event_generation: u64,
) -> TauriXbxEngineRuntime {
    let event_bridge = TauriEngineEventBridge {
        app_handle: app_handle.clone(),
        state: Default::default(),
        last_runtime_event,
        runtime_trace: runtime_trace.clone(),
    };
    let input_backend = Box::new(OhMyGamepadXbxEngineInputBackend::new());
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
    XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TauriXbxEngineHostBridge {
            app_handle,
            native_video,
            runtime_trace: runtime_trace.clone(),
            host_present_route,
            cancellation_epoch,
            rumble_worker,
        },
        TauriXbxEngineEventSink {
            bridge: Arc::new(StdMutex::new(event_bridge)),
            generation: event_generation,
            active_generation: active_event_generation,
        },
        media_backend,
    )
}

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
    app_handle: AppHandle,
    last_runtime_event: Arc<StdMutex<Option<serde_json::Value>>>,
    native_video: NativeVideoRegistryRef,
    runtime_trace: RuntimeTraceRecorderRef,
    /// 与 `XbxEngineRuntime` 快照同步，供 `NativeVideoHostRenderFramePush` 在 renderer 线程读取。
    host_present_route: Arc<StdMutex<XbxEngineHostPresentRoute>>,
    /// `statsSnapshot` / `observabilitySnapshot` 最小间隔（毫秒），可由设置 `runtime_trace_mode` 运行时更新。
    stats_snapshot_interval_ms: Arc<AtomicU64>,
    last_stats_trace_at: StdMutex<Option<Instant>>,
    last_trace_observation_at: StdMutex<Option<Instant>>,
    last_trace_observation: StdMutex<RuntimeTraceObservationState>,
    active_session_id: StdMutex<Option<String>>,
    pending_control_count: AtomicU64,
    last_runtime_lock_wait_trace_at: StdMutex<Option<Instant>>,
    last_tick_lock_busy_trace_at: StdMutex<Option<Instant>>,
    cancellation_epoch: Arc<AtomicU64>,
    active_event_generation: Arc<AtomicU64>,
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
        let host_present_route = Arc::new(StdMutex::new(XbxEngineHostPresentRoute::default()));
        let active_event_generation =
            Arc::new(AtomicU64::new(XBXENGINE_INITIAL_RUNTIME_GENERATION));
        let rumble_worker =
            GamepadRumbleWorkerHandle::new(app_handle.clone(), runtime_trace.clone());
        let runtime = create_tauri_runtime(
            app_handle.clone(),
            last_runtime_event.clone(),
            native_video.clone(),
            runtime_trace.clone(),
            host_present_route.clone(),
            cancellation_epoch.clone(),
            rumble_worker.clone(),
            active_event_generation.clone(),
            XBXENGINE_INITIAL_RUNTIME_GENERATION,
        );
        Self {
            runtime: StdMutex::new(runtime),
            app_handle,
            last_runtime_event,
            native_video,
            runtime_trace,
            host_present_route,
            stats_snapshot_interval_ms: Arc::new(AtomicU64::new(
                stats_snapshot_interval.as_millis() as u64,
            )),
            last_stats_trace_at: StdMutex::new(None),
            last_trace_observation_at: StdMutex::new(None),
            last_trace_observation: StdMutex::new(RuntimeTraceObservationState::default()),
            active_session_id: StdMutex::new(None),
            pending_control_count: AtomicU64::new(0),
            last_runtime_lock_wait_trace_at: StdMutex::new(None),
            last_tick_lock_busy_trace_at: StdMutex::new(None),
            cancellation_epoch,
            active_event_generation,
            rumble_worker,
        }
    }

    pub fn set_stats_snapshot_interval(&self, interval: Duration) {
        self.stats_snapshot_interval_ms
            .store(interval.as_millis() as u64, Ordering::Relaxed);
    }

    fn enter_pending_control(&self) -> PendingControlGuard<'_> {
        self.pending_control_count.fetch_add(1, Ordering::SeqCst);
        PendingControlGuard(&self.pending_control_count)
    }

    fn should_record_lock_trace(last_trace_at: &StdMutex<Option<Instant>>) -> bool {
        let Ok(mut last_trace_at) = last_trace_at.lock() else {
            return true;
        };
        let now = Instant::now();
        let due = last_trace_at
            .map(|last| now.duration_since(last) >= XBXENGINE_RUNTIME_LOCK_TRACE_INTERVAL)
            .unwrap_or(true);
        if due {
            *last_trace_at = Some(now);
        }
        due
    }

    fn record_control_waiting_for_runtime_lock(
        &self,
        command: &XbxEngineControlCommandDto,
        wait_ms: u128,
    ) {
        if !Self::should_record_lock_trace(&self.last_runtime_lock_wait_trace_at) {
            return;
        }
        self.runtime_trace.record_event(
            "xbxengine-host",
            "runtimeControlWaitingForRuntimeLock",
            extract_command_session_id(command).as_deref(),
            serde_json::json!({
                "command": control_command_name(command),
                "sessionId": extract_command_session_id(command),
                "viewportId": extract_command_viewport_id(command),
                "targetType": extract_command_target_type(command),
                "waitMs": wait_ms,
                "pendingControlCount": self.pending_control_count.load(Ordering::SeqCst),
            }),
        );
    }

    fn record_tick_skipped_for_runtime_lock(&self, stage: &'static str, reason: &'static str) {
        if !Self::should_record_lock_trace(&self.last_tick_lock_busy_trace_at) {
            return;
        }
        let session_id = self
            .active_session_id
            .lock()
            .ok()
            .and_then(|guard| guard.clone());
        self.runtime_trace.record_event(
            "xbxengine-host",
            "runtimeTickSkippedRuntimeLockBusy",
            session_id.as_deref(),
            serde_json::json!({
                "stage": stage,
                "reason": reason,
                "pendingControlCount": self.pending_control_count.load(Ordering::SeqCst),
            }),
        );
    }

    fn sync_host_present_route_from_runtime(&self, runtime: &TauriXbxEngineRuntime) {
        let snap = runtime.snapshot();
        if let Ok(mut guard) = self.host_present_route.lock() {
            guard.viewport = snap.viewport.clone();
            guard.surface_id = snap.surface_id.clone();
        }
    }

    fn create_runtime_for_next_generation(&self) -> (TauriXbxEngineRuntime, u64) {
        let generation = self
            .active_event_generation
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        if let Ok(mut last_runtime_event) = self.last_runtime_event.lock() {
            *last_runtime_event = None;
        }
        let runtime = create_tauri_runtime(
            self.app_handle.clone(),
            self.last_runtime_event.clone(),
            self.native_video.clone(),
            self.runtime_trace.clone(),
            self.host_present_route.clone(),
            self.cancellation_epoch.clone(),
            self.rumble_worker.clone(),
            self.active_event_generation.clone(),
            generation,
        );
        (runtime, generation)
    }

    pub fn apply_control(
        &self,
        command: XbxEngineControlCommandDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        let _pending_control = self.enter_pending_control();
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
        if matches!(command, XbxEngineControlCommandDto::StopRuntime { .. }) {
            return self.apply_stop_runtime_control(command);
        }
        let lock_started_at = Instant::now();
        let mut runtime = match self.runtime.try_lock() {
            Ok(runtime) => runtime,
            Err(TryLockError::WouldBlock) => {
                self.record_control_waiting_for_runtime_lock(
                    &command,
                    lock_started_at.elapsed().as_millis(),
                );
                self.runtime
                    .lock()
                    .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRuntimeLockPoisoned"))?
            }
            Err(TryLockError::Poisoned(_)) => {
                return Err(XbxEngineRuntimeError::new("xbxEngineRuntimeLockPoisoned"));
            }
        };
        record_control_apply_trace(
            &self.runtime_trace,
            &command,
            "lockAcquired",
            Some(lock_started_at.elapsed().as_millis()),
            None,
        );
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

    fn apply_stop_runtime_control(
        &self,
        command: XbxEngineControlCommandDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        let lock_started_at = Instant::now();
        let mut runtime = match self.runtime.try_lock() {
            Ok(runtime) => runtime,
            Err(TryLockError::WouldBlock) => {
                self.record_control_waiting_for_runtime_lock(
                    &command,
                    lock_started_at.elapsed().as_millis(),
                );
                self.runtime
                    .lock()
                    .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRuntimeLockPoisoned"))?
            }
            Err(TryLockError::Poisoned(_)) => {
                return Err(XbxEngineRuntimeError::new("xbxEngineRuntimeLockPoisoned"));
            }
        };
        record_control_apply_trace(
            &self.runtime_trace,
            &command,
            "lockAcquired",
            Some(lock_started_at.elapsed().as_millis()),
            None,
        );

        let replacement_started_at = Instant::now();
        let (replacement_runtime, replacement_generation) =
            self.create_runtime_for_next_generation();
        if let Err(error) = runtime.apply_control(XbxEngineControlCommandDto::DetachViewport) {
            self.runtime_trace.record_event(
                "xbxengine-host",
                "runtimeStopPreDetachFailed",
                None,
                serde_json::json!({
                    "error": error.to_string(),
                }),
            );
        }

        let mut draining_runtime = std::mem::replace(&mut *runtime, replacement_runtime);
        self.sync_host_present_route_from_runtime(&runtime);
        drop(runtime);

        let runtime_trace = self.runtime_trace.clone();
        let drain_command = command.clone();
        std::thread::spawn(move || {
            let drain_started_at = Instant::now();
            runtime_trace.record_event(
                "xbxengine-host",
                "runtimeStopDrainStarted",
                None,
                serde_json::json!({
                    "replacementGeneration": replacement_generation,
                }),
            );
            let result = draining_runtime.apply_control(drain_command);
            runtime_trace.record_event(
                "xbxengine-host",
                "runtimeStopDrainCompleted",
                None,
                serde_json::json!({
                    "durationMs": drain_started_at.elapsed().as_millis(),
                    "ok": result.is_ok(),
                    "error": result.as_ref().err().map(ToString::to_string),
                    "replacementGeneration": replacement_generation,
                }),
            );
        });

        record_control_apply_trace(
            &self.runtime_trace,
            &command,
            "completed",
            Some(replacement_started_at.elapsed().as_millis()),
            None,
        );
        Ok(())
    }

    pub fn tick(&self) -> Result<(), XbxEngineRuntimeError> {
        if self.pending_control_count.load(Ordering::SeqCst) > 0 {
            self.record_tick_skipped_for_runtime_lock("beforeViewport", "controlPending");
            return Ok(());
        }
        let viewport_id = match self.try_current_viewport_id_for_tick()? {
            Some(viewport_id) => viewport_id,
            None => return Ok(()),
        };
        let native_video_feedback = self.collect_native_video_host_feedback(viewport_id.as_deref());
        let stats_snapshot = {
            if self.pending_control_count.load(Ordering::SeqCst) > 0 {
                self.record_tick_skipped_for_runtime_lock("beforeLock", "controlPending");
                return Ok(());
            }
            let mut runtime = match self.runtime.try_lock() {
                Ok(runtime) => runtime,
                Err(TryLockError::WouldBlock) => {
                    self.record_tick_skipped_for_runtime_lock("tryLock", "runtimeBusy");
                    return Ok(());
                }
                Err(TryLockError::Poisoned(_)) => {
                    return Err(XbxEngineRuntimeError::new("xbxEngineRuntimeLockPoisoned"));
                }
            };
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
        let observation_interval =
            trace_observation_tick_interval(self.runtime_trace.trace_profile());
        let should_project = self
            .last_trace_observation_at
            .lock()
            .ok()
            .map(|mut last| {
                let now = Instant::now();
                let due = last
                    .map(|previous| now.duration_since(previous) >= observation_interval)
                    .unwrap_or(true);
                if due {
                    *last = Some(now);
                }
                due
            })
            .unwrap_or(true);
        if should_project {
            self.record_runtime_trace_observations(session_id.as_deref(), &stats_snapshot);
        }
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

    fn try_current_viewport_id_for_tick(
        &self,
    ) -> Result<Option<Option<String>>, XbxEngineRuntimeError> {
        match self.runtime.try_lock() {
            Ok(runtime) => Ok(Some(
                runtime
                    .snapshot()
                    .viewport
                    .as_ref()
                    .map(|viewport| viewport.viewport_id.clone()),
            )),
            Err(TryLockError::WouldBlock) => {
                self.record_tick_skipped_for_runtime_lock("viewportLock", "runtimeBusy");
                Ok(None)
            }
            Err(TryLockError::Poisoned(_)) => {
                Err(XbxEngineRuntimeError::new("xbxEngineRuntimeLockPoisoned"))
            }
        }
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
        let started_at = Instant::now();
        let request_id = XBXENGINE_HOST_EXCHANGE_OFFER_REQUEST_SEQ.fetch_add(1, Ordering::Relaxed);
        let trace_session_id = session_id.clone();
        let trace_channel = channel.clone();
        self.runtime_trace.record_event(
            "xbxengine-host",
            "exchangeOfferRequested",
            Some(&session_id),
            serde_json::json!({
                "requestId": request_id,
                "channel": channel,
                "restart": restart,
                "offerSdp": sdp,
                "offerSdpCapabilities": summarize_sdp_capabilities(&sdp),
                "timeoutMs": XBXENGINE_HOST_EXCHANGE_OFFER_TIMEOUT.as_millis(),
            }),
        );
        let exchange = state
            .streaming
            .exchange_offer(StreamingExchangeOfferParams {
                session_id,
                channel: Some(channel),
                sdp,
                restart,
            });
        let result = match tauri::async_runtime::block_on(exchange_offer_with_timeout(
            exchange,
            XBXENGINE_HOST_EXCHANGE_OFFER_TIMEOUT,
        )) {
            Ok(result) => result,
            Err(error) => {
                let failure_kind = exchange_offer_failure_kind(&error);
                self.runtime_trace.record_event(
                    "xbxengine-host",
                    "exchangeOfferFailed",
                    Some(&trace_session_id),
                    serde_json::json!({
                        "requestId": request_id,
                        "channel": trace_channel,
                        "restart": restart,
                        "durationMs": started_at.elapsed().as_millis(),
                        "timeoutMs": XBXENGINE_HOST_EXCHANGE_OFFER_TIMEOUT.as_millis(),
                        "failureKind": failure_kind,
                        "phase": "exchangeOffer",
                        "answerReceived": false,
                        "error": error.to_string(),
                    }),
                );
                return Err(map_app_error("exchangeOffer")(error));
            }
        };
        self.runtime_trace.record_event(
            "xbxengine-host",
            "exchangeOfferResult",
            Some(&trace_session_id),
            serde_json::json!({
                "requestId": request_id,
                "channel": trace_channel,
                "restart": restart,
                "durationMs": started_at.elapsed().as_millis(),
                "answerReceived": true,
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
                let accepted =
                    tauri::async_runtime::block_on(state.streaming.send_keepalive(session_id))
                        .map_err(map_app_error("keepAliveRemoteSession"))?;
                self.runtime_trace.record_event(
                    "xbxengine-host",
                    "keepAliveRemoteSession",
                    None,
                    serde_json::json!({ "accepted": accepted }),
                );
                if !accepted {
                    return Err(XbxEngineRuntimeError::new(
                        "keepAliveRemoteSession:streaming:SessionNotActive",
                    ));
                }
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
    generation: u64,
    active_generation: Arc<AtomicU64>,
}

impl XbxEngineEventSink for TauriXbxEngineEventSink {
    fn emit(&mut self, event: XbxEngineRuntimeEventDto) {
        if self.active_generation.load(Ordering::SeqCst) != self.generation {
            return;
        }
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

async fn exchange_offer_with_timeout<F>(
    exchange: F,
    timeout: Duration,
) -> AppResult<crate::mods::streaming::StreamingExchangeOfferResult>
where
    F: Future<Output = AppResult<crate::mods::streaming::StreamingExchangeOfferResult>>,
{
    match tokio::time::timeout(timeout, exchange).await {
        Ok(result) => result,
        Err(_) => Err(AppError::XbxEngine(format!(
            "exchange offer timed out after {}ms",
            timeout.as_millis()
        ))),
    }
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

fn control_command_name(command: &XbxEngineControlCommandDto) -> &'static str {
    match command {
        XbxEngineControlCommandDto::StartRuntime { .. } => "StartRuntime",
        XbxEngineControlCommandDto::StopRuntime { .. } => "StopRuntime",
        XbxEngineControlCommandDto::RequestReconnect { .. } => "RequestReconnect",
        XbxEngineControlCommandDto::AttachViewport { .. } => "AttachViewport",
        XbxEngineControlCommandDto::DetachViewport => "DetachViewport",
        XbxEngineControlCommandDto::ApplyDisplayState { .. } => "ApplyDisplayState",
        XbxEngineControlCommandDto::SetAudioVolume { .. } => "SetAudioVolume",
        XbxEngineControlCommandDto::StartMicrophone => "StartMicrophone",
        XbxEngineControlCommandDto::StopMicrophone => "StopMicrophone",
        XbxEngineControlCommandDto::PressControllerButton { .. } => "PressControllerButton",
        XbxEngineControlCommandDto::SetKeyboardPointerEnabled { .. } => "SetKeyboardPointerEnabled",
        XbxEngineControlCommandDto::PushKeyboardPointerInput { .. } => "PushKeyboardPointerInput",
    }
}

fn exchange_offer_failure_kind(error: &AppError) -> &'static str {
    exchange_offer_failure_kind_from_message(&error.to_string())
}

fn exchange_offer_failure_kind_from_message(message: &str) -> &'static str {
    if message.contains("timed out") {
        "timeout"
    } else {
        "error"
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
        (XbxEngineControlCommandDto::StopRuntime { .. }, "entered") => {
            Some("runtimeStopApplyEntered")
        }
        (XbxEngineControlCommandDto::StopRuntime { .. }, "lockAcquired") => {
            Some("runtimeStopRuntimeLockAcquired")
        }
        (XbxEngineControlCommandDto::StopRuntime { .. }, "completed") => {
            Some("runtimeStopApplyCompleted")
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
    use std::time::Duration;

    use super::{
        control_apply_event_name, exchange_offer_failure_kind_from_message,
        exchange_offer_with_timeout, summarize_sdp_capabilities,
        update_host_present_route_on_attach, update_host_present_route_on_detach,
        NativeVideoHostRenderFramePush,
    };
    use crate::mods::native_video::NativeVideoRegistry;
    use crate::mods::runtime_trace::{RuntimeTraceRecorder, RuntimeTraceRecorderRef};
    use crate::mods::streaming::{StreamingAnswerPayload, StreamingExchangeOfferResult};
    use xbxengine::{
        XbxEngineHostPresentRoute, XbxEngineRenderFrame, XbxEngineRenderPixelData,
        XbxHostRenderFramePush, XbxHostRenderFramePushOutcome,
    };
    use xbxengine_protocol::{XbxEngineControlCommandDto, XbxEngineViewportDto};

    fn test_runtime_trace() -> RuntimeTraceRecorderRef {
        Arc::new(RuntimeTraceRecorder::new_with_mode("off").expect("runtime trace"))
    }

    fn test_render_frame(frame_seq: u64) -> XbxEngineRenderFrame {
        XbxEngineRenderFrame {
            width: 2,
            height: 2,
            frame_seq,
            rendered_at_ms: 1_000.0,
            rtp_timestamp: Some(frame_seq as u32),
            recovery_epoch_tag: None,
            recovery_owner_rtp_timestamp: None,
            is_keyframe: frame_seq == 1,
            frame_recovery_disposition: None,
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::from(vec![0_u8; 16].into_boxed_slice()),
            },
        }
    }

    #[test]
    fn stop_runtime_control_trace_events_are_named() {
        let command = XbxEngineControlCommandDto::StopRuntime {
            reason: Some("menuActionExit".to_string()),
        };

        assert_eq!(
            control_apply_event_name(&command, "entered"),
            Some("runtimeStopApplyEntered")
        );
        assert_eq!(
            control_apply_event_name(&command, "lockAcquired"),
            Some("runtimeStopRuntimeLockAcquired")
        );
        assert_eq!(
            control_apply_event_name(&command, "completed"),
            Some("runtimeStopApplyCompleted")
        );
    }

    #[tokio::test]
    async fn exchange_offer_with_timeout_returns_error_when_future_stalls() {
        let result = exchange_offer_with_timeout(
            async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok(StreamingExchangeOfferResult {
                    answer: StreamingAnswerPayload {
                        sdp: "answer".to_string(),
                        message_type: None,
                    },
                })
            },
            Duration::from_millis(5),
        )
        .await;

        let error = result.expect_err("exchange offer should time out");
        assert!(error.to_string().contains("exchange offer timed out"));
    }

    #[test]
    fn exchange_offer_failure_kind_marks_timeout() {
        assert_eq!(
            exchange_offer_failure_kind_from_message("exchange offer timed out after 20000ms"),
            "timeout"
        );
        assert_eq!(
            exchange_offer_failure_kind_from_message("remote session rejected offer"),
            "error"
        );
    }

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

    #[test]
    fn native_host_push_returns_when_route_lock_is_busy() {
        let native_video = Arc::new(StdMutex::new(NativeVideoRegistry::default()));
        let route = Arc::new(StdMutex::new(XbxEngineHostPresentRoute {
            viewport: Some(XbxEngineViewportDto {
                viewport_id: "vp-1".to_string(),
            }),
            surface_id: Some("wgpu:surface-1".to_string()),
        }));
        let _route_guard = route.lock().expect("route lock");
        let push = NativeVideoHostRenderFramePush {
            native_video,
            route: route.clone(),
            runtime_trace: test_runtime_trace(),
        };

        let outcome = push.push_render_frame_for_host_present(test_render_frame(42));

        assert_eq!(outcome, XbxHostRenderFramePushOutcome::RouteUnavailable);
    }

    #[test]
    fn native_host_push_returns_when_registry_lock_is_busy() {
        let native_video = Arc::new(StdMutex::new(NativeVideoRegistry::default()));
        let route = Arc::new(StdMutex::new(XbxEngineHostPresentRoute {
            viewport: Some(XbxEngineViewportDto {
                viewport_id: "vp-1".to_string(),
            }),
            surface_id: Some("wgpu:surface-1".to_string()),
        }));
        let _registry_guard = native_video.lock().expect("registry lock");
        let push = NativeVideoHostRenderFramePush {
            native_video: native_video.clone(),
            route,
            runtime_trace: test_runtime_trace(),
        };

        let outcome = push.push_render_frame_for_host_present(test_render_frame(43));

        assert_eq!(outcome, XbxHostRenderFramePushOutcome::RegistryUnavailable);
    }
}
