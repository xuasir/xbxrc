use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
};

use ohmygamepad_core::{
    ArcSwap, DeviceProfile, HapticsProvider, InputCoreConfig, InputProcessingMode,
    InputRuntimeError, InputRuntimeHandle, NoopStreamSink, NoopUiSink,
};
use ohmygamepad_protocol::{
    LogicalPadBindingDto, LogicalPadId, LogicalPadSnapshotDto, LogicalPadStateDto,
    MultiControllerSamplingModeDto, MultiControllerSamplingStrategyDto, OhMyGamepadBindingModeDto,
    OhMyGamepadDeviceDto, OhMyGamepadInputRouteDto, OhMyGamepadKeyboardMappingDto,
    OhMyGamepadRumbleRequestDto, OhMyGamepadRumbleResultDto, OhMyGamepadRumbleTargetDto,
    OhMyGamepadRuntimeSnapshotDto, OhMyGamepadSamplingConfigDto, OhMyGamepadSamplingPresetDto,
    OhMyGamepadServiceCommandDto, SimulatedGamepadDescriptorDto,
};

use crate::service_keyboard::{
    spawn_keyboard_listener_thread, state_snapshot_event, ServiceKeyboardListenerHandle,
    KEYBOARD_FALLBACK_DEVICE_ID, KEYBOARD_FALLBACK_DEVICE_NAME,
};
use crate::service_rumble::{
    prepare_rumble_dispatch, resolve_connected_target_devices,
    rumble_backend_from_haptics_provider, PreparedRumbleRequest, ServiceRumbleBackend,
};
use crate::service_source::OhMyGamepadServiceSource;
use crate::{
    spawn_sdl3_input_runtime_with_source, OhMyGamepadDesktopKeyboardListenerConfig, RealSdl3Source,
    Sdl3BackendConfig, Sdl3DeviceDescriptor, Sdl3InputEvent, Sdl3InputEventKind, Sdl3Source,
    Sdl3SourceInitError,
};

const EMPTY_SAMPLING_DEVICE_ID: &str = "__service:none__";
const SERVICE_SYNC_RETRY_COUNT: usize = 40;
const SERVICE_SYNC_RETRY_DELAY_MS: u64 = 2;

pub type MultiControllerSamplingMode = MultiControllerSamplingModeDto;
pub type MultiControllerSamplingStrategy = MultiControllerSamplingStrategyDto;
pub type SimulatedGamepadDescriptor = SimulatedGamepadDescriptorDto;

#[derive(Clone, Debug, PartialEq)]
pub struct OhMyGamepadServiceConfig {
    pub core: InputCoreConfig,
    pub backend: Sdl3BackendConfig,
    pub sampling_strategy: MultiControllerSamplingStrategy,
    pub desktop_keyboard: Option<OhMyGamepadDesktopKeyboardListenerConfig>,
}

impl Default for OhMyGamepadServiceConfig {
    fn default() -> Self {
        Self {
            core: InputCoreConfig::default(),
            backend: Sdl3BackendConfig::default(),
            sampling_strategy: MultiControllerSamplingStrategy::default(),
            desktop_keyboard: Some(OhMyGamepadDesktopKeyboardListenerConfig::default()),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum OhMyGamepadServiceError {
    Runtime(InputRuntimeError),
    InputChannelClosed,
}

impl From<InputRuntimeError> for OhMyGamepadServiceError {
    fn from(value: InputRuntimeError) -> Self {
        Self::Runtime(value)
    }
}

struct OhMyGamepadServiceState {
    sampling_strategy: MultiControllerSamplingStrategy,
    virtual_input_tx: Sender<Vec<Sdl3InputEvent>>,
    virtual_descriptors: HashMap<String, SimulatedGamepadDescriptor>,
    desktop_keyboard_config: Option<OhMyGamepadDesktopKeyboardListenerConfig>,
    keyboard_listener: Option<ServiceKeyboardListenerHandle>,
    source_handle: Option<crate::Sdl3RumbleHandle>,
    rumble_backend: Option<Box<dyn ServiceRumbleBackend>>,
}

/// 主窗口 route hints，由 Tauri 写入；在 host enrich 中派生统一三态 `input_route`。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ShellWindowRouteHints {
    /// 壳层当前是否拥有输入归属。
    pub shell_app_active: bool,
}

/// 统一输入路由（Phase 4 引入）：比旧 gate 更完整的三态真相源。
///
/// ```text
/// foreground  = shell_window_route_hints.shell_app_active（= `!minimized`）
/// route =
///   !foreground                     → Blocked  // 旧 gate Closed
///   stream_forwarding && !ui_capture → Stream
///   otherwise                         → Ui
/// ```
pub fn derive_input_route(
    hints: &ShellWindowRouteHints,
    stream_forwarding: bool,
    ui_capture: bool,
) -> (OhMyGamepadInputRouteDto, String) {
    if !hints.shell_app_active {
        return (
            OhMyGamepadInputRouteDto::Blocked,
            "route:blocked-shell-background".to_owned(),
        );
    }
    if stream_forwarding && !ui_capture {
        (OhMyGamepadInputRouteDto::Stream, "route:stream".to_owned())
    } else {
        (OhMyGamepadInputRouteDto::Ui, "route:ui".to_owned())
    }
}

pub struct OhMyGamepadService {
    processing_mode: InputProcessingMode,
    runtime: InputRuntimeHandle,
    state: Mutex<OhMyGamepadServiceState>,
    /// When true, RTC input loop may forward logical pad samples to the stream transport.
    stream_pad_forwarding: Arc<AtomicBool>,
    /// 前端 overlay / 菜单 / 诊断页等 UI 正在捕获输入：route 切回 Ui。
    ui_capture: Arc<AtomicBool>,
    shell_window_route_hints: Arc<Mutex<ShellWindowRouteHints>>,
    /// 最近一次派生的 input_route discriminant（Phase 4 引入，取代旧 gate）。
    latest_input_route: Arc<AtomicU8>,
}

impl OhMyGamepadService {
    pub fn spawn(config: OhMyGamepadServiceConfig) -> Result<Self, Sdl3SourceInitError> {
        let (physical_source, rumble_handle) = RealSdl3Source::new(config.backend.clone())?;
        Ok(Self::spawn_with_source_and_rumble(
            config,
            physical_source,
            rumble_handle.clone(),
            rumble_handle.map(|handle| Box::new(handle) as Box<dyn ServiceRumbleBackend>),
        ))
    }

    pub fn spawn_with_source<TSource>(
        config: OhMyGamepadServiceConfig,
        physical_source: TSource,
    ) -> Self
    where
        TSource: Sdl3Source + Send + 'static,
    {
        Self::spawn_with_source_and_rumble(config, physical_source, None, None)
    }

    pub fn spawn_with_haptics_provider(
        config: OhMyGamepadServiceConfig,
        haptics_provider: Box<dyn HapticsProvider>,
    ) -> Result<Self, Sdl3SourceInitError> {
        let (physical_source, _sdl3_rumble_handle) = RealSdl3Source::new(config.backend.clone())?;
        Ok(Self::spawn_with_source_and_rumble(
            config,
            physical_source,
            None,
            Some(rumble_backend_from_haptics_provider(haptics_provider)),
        ))
    }

    fn spawn_with_source_and_rumble<TSource>(
        config: OhMyGamepadServiceConfig,
        physical_source: TSource,
        source_handle: Option<crate::Sdl3RumbleHandle>,
        rumble_backend: Option<Box<dyn ServiceRumbleBackend>>,
    ) -> Self
    where
        TSource: Sdl3Source + Send + 'static,
    {
        let (virtual_input_tx, virtual_input_rx) = mpsc::channel();
        let source = OhMyGamepadServiceSource::new(
            physical_source,
            virtual_input_rx,
            config.sampling_strategy.enable_keyboard_fallback,
            now_ms,
        );

        let processing_mode = config.core.processing_mode;
        let runtime = spawn_sdl3_input_runtime_with_source(
            normalize_core_config(config.core, &config.sampling_strategy),
            config.backend,
            source,
            NoopUiSink,
            NoopStreamSink,
        );
        let desktop_keyboard_config = config.desktop_keyboard.clone();
        let keyboard_listener = if config.sampling_strategy.enable_keyboard_fallback {
            desktop_keyboard_config.clone().map(|keyboard_config| {
                spawn_keyboard_listener_thread(keyboard_config, virtual_input_tx.clone(), now_ms)
            })
        } else {
            None
        };

        Self {
            processing_mode,
            runtime,
            state: Mutex::new(OhMyGamepadServiceState {
                sampling_strategy: config.sampling_strategy,
                virtual_input_tx,
                virtual_descriptors: HashMap::new(),
                desktop_keyboard_config,
                keyboard_listener,
                source_handle,
                rumble_backend,
            }),
            stream_pad_forwarding: Arc::new(AtomicBool::new(false)),
            ui_capture: Arc::new(AtomicBool::new(false)),
            shell_window_route_hints: Arc::new(Mutex::new(ShellWindowRouteHints::default())),
            latest_input_route: Arc::new(AtomicU8::new(
                OhMyGamepadInputRouteDto::Blocked.discriminant(),
            )),
        }
    }

    /// 用户手动「重新扫描手柄」：唯一允许关闭并重开设备句柄的入口。
    pub fn rescan_devices(&self) -> Result<(), InputRuntimeError> {
        let source_handle = self
            .state
            .lock()
            .expect("lock service state")
            .source_handle
            .clone();
        if let Some(source_handle) = source_handle {
            if let Err(error) = source_handle.rescan_devices() {
                log::warn!("ohmygamepad_rescan_devices_failed error={}", error);
                return Err(InputRuntimeError::SourceCommandFailed);
            }
        }
        self.runtime.refresh_snapshot()
    }

    fn ensure_keyboard_listener_running(&self, reason: &str) -> bool {
        let mut state = self.state.lock().expect("lock service state");
        if !state.sampling_strategy.enable_keyboard_fallback {
            return false;
        }
        let Some(config) = state.desktop_keyboard_config.clone() else {
            return false;
        };
        if let Some(listener) = state.keyboard_listener.as_ref() {
            if !listener.is_finished() {
                return false;
            }
        }

        if let Some(listener) = state.keyboard_listener.take() {
            listener.shutdown();
        }
        state.keyboard_listener = Some(spawn_keyboard_listener_thread(
            config,
            state.virtual_input_tx.clone(),
            now_ms,
        ));
        log::info!(
            "ohmygamepad_keyboard_fallback_listener_restarted reason={}",
            reason
        );
        true
    }

    pub fn snapshot(&self) -> Result<OhMyGamepadRuntimeSnapshotDto, InputRuntimeError> {
        self.snapshot_with_strategy_sync()
    }

    pub fn subscribe_runtime_snapshot(&self) -> Receiver<OhMyGamepadRuntimeSnapshotDto> {
        self.runtime.subscribe_runtime_snapshot()
    }

    /// 无锁读取最新 pad 状态的原子单元；高频读方（如串流输入环）直接 load。
    pub fn get_latest_pad_states(&self) -> Arc<ArcSwap<Vec<LogicalPadSnapshotDto>>> {
        self.runtime.latest_pad_states()
    }

    pub fn refresh_snapshot(&self) -> Result<(), InputRuntimeError> {
        self.runtime.refresh_snapshot()
    }

    pub fn set_stream_pad_forwarding(&self, enabled: bool) {
        self.stream_pad_forwarding.store(enabled, Ordering::Relaxed);
        self.refresh_input_route_cache();
    }

    pub fn stream_pad_forwarding(&self) -> bool {
        self.stream_pad_forwarding.load(Ordering::Relaxed)
    }

    fn refresh_input_route_cache(&self) {
        let hints = *self
            .shell_window_route_hints
            .lock()
            .expect("lock shell window route hints");
        let (route, _) = derive_input_route(
            &hints,
            self.stream_pad_forwarding.load(Ordering::Relaxed),
            self.ui_capture.load(Ordering::Relaxed),
        );
        self.latest_input_route
            .store(route.discriminant(), Ordering::Relaxed);
    }

    pub fn set_shell_window_route_hints(&self, hints: ShellWindowRouteHints) {
        *self
            .shell_window_route_hints
            .lock()
            .expect("lock shell window route hints") = hints;
        self.refresh_input_route_cache();
        let _ = self.runtime.refresh_snapshot();
    }

    /// 无锁读取最近一次派生的三态输入路由（Phase 4 真相源）。
    pub fn peek_input_route(&self) -> OhMyGamepadInputRouteDto {
        OhMyGamepadInputRouteDto::from_discriminant(self.latest_input_route.load(Ordering::Relaxed))
    }

    /// 前端 overlay / 菜单 / 诊断页等上报"正在捕获输入"：route 切回 Ui。
    /// Release 后前端再次 set_ui_capture(false) 切回正常 Stream 判定。
    pub fn set_ui_capture(&self, capturing: bool) {
        self.ui_capture.store(capturing, Ordering::Relaxed);
        self.refresh_input_route_cache();
    }

    pub fn shell_window_route_hints(&self) -> ShellWindowRouteHints {
        *self
            .shell_window_route_hints
            .lock()
            .expect("lock shell window route hints")
    }

    pub fn list_devices(&self) -> Result<Vec<OhMyGamepadDeviceDto>, InputRuntimeError> {
        Ok(self.snapshot()?.devices)
    }

    pub fn discover_devices(&self) -> Result<Vec<OhMyGamepadDeviceDto>, InputRuntimeError> {
        self.list_devices()
    }

    pub fn get_device(
        &self,
        device_id: &str,
    ) -> Result<Option<OhMyGamepadDeviceDto>, InputRuntimeError> {
        Ok(self
            .list_devices()?
            .into_iter()
            .find(|device| device.device_id == device_id))
    }

    pub fn sampling_strategy(&self) -> MultiControllerSamplingStrategy {
        self.state
            .lock()
            .expect("lock service state")
            .sampling_strategy
            .clone()
    }

    pub fn set_sampling(
        &self,
        sampling: OhMyGamepadSamplingConfigDto,
    ) -> Result<(), InputRuntimeError> {
        self.runtime.update_sampling(sampling)
    }

    pub fn set_sampling_preset(
        &self,
        preset: OhMyGamepadSamplingPresetDto,
    ) -> Result<(), InputRuntimeError> {
        self.set_sampling(OhMyGamepadSamplingConfigDto::from_preset(preset))
    }

    pub fn set_sampling_strategy(
        &self,
        strategy: MultiControllerSamplingStrategy,
    ) -> Result<(), InputRuntimeError> {
        {
            let mut state = self.state.lock().expect("lock service state");
            state.sampling_strategy = strategy;
        }
        let _ = self.snapshot_with_strategy_sync()?;
        Ok(())
    }

    pub fn set_primary_sampling_device(
        &self,
        device_id: Option<String>,
    ) -> Result<(), InputRuntimeError> {
        let mut strategy = self.sampling_strategy();
        strategy.mode = if device_id.is_some() {
            MultiControllerSamplingMode::PrimaryPreferred
        } else {
            MultiControllerSamplingMode::Merge
        };
        strategy.primary_device_id = device_id;
        self.set_sampling_strategy(strategy)
    }

    pub fn pause_sampling_device(&self, device_id: &str) -> Result<(), InputRuntimeError> {
        let mut strategy = self.sampling_strategy();
        if !strategy.paused_device_ids.iter().any(|id| id == device_id) {
            strategy.paused_device_ids.push(device_id.to_owned());
            strategy.paused_device_ids.sort();
        }
        self.set_sampling_strategy(strategy)
    }

    pub fn resume_sampling_device(&self, device_id: &str) -> Result<(), InputRuntimeError> {
        let mut strategy = self.sampling_strategy();
        strategy.paused_device_ids.retain(|id| id != device_id);
        self.set_sampling_strategy(strategy)
    }

    pub fn rebind_logical_pad(
        &self,
        binding: LogicalPadBindingDto,
    ) -> Result<(), InputRuntimeError> {
        self.runtime.rebind_logical_pad(binding)
    }

    pub fn replace_device_profiles(
        &self,
        profiles: Vec<DeviceProfile>,
    ) -> Result<(), InputRuntimeError> {
        self.runtime.replace_device_profiles(profiles)
    }

    pub fn connect_simulated_gamepad(
        &self,
        descriptor: SimulatedGamepadDescriptor,
    ) -> Result<(), OhMyGamepadServiceError> {
        let device_id = descriptor.device_id.clone();
        let event = simulated_connection_event(&descriptor, true);
        let tx = {
            let mut state = self.state.lock().expect("lock service state");
            state
                .virtual_descriptors
                .insert(descriptor.device_id.clone(), descriptor);
            state.virtual_input_tx.clone()
        };
        send_virtual_events(&tx, vec![event])?;
        let _ = self.wait_for_device_presence(&device_id, true)?;
        Ok(())
    }

    pub fn disconnect_simulated_gamepad(
        &self,
        device_id: &str,
    ) -> Result<(), OhMyGamepadServiceError> {
        let event = simulated_connection_event(
            &SimulatedGamepadDescriptor {
                device_id: device_id.to_owned(),
                name: device_id.to_owned(),
                ..SimulatedGamepadDescriptor::default()
            },
            false,
        );
        let tx = {
            let mut state = self.state.lock().expect("lock service state");
            state.virtual_descriptors.remove(device_id);
            state.virtual_input_tx.clone()
        };
        send_virtual_events(&tx, vec![event])?;
        let _ = self.wait_for_device_presence(device_id, false)?;
        Ok(())
    }

    pub fn submit_simulated_state(
        &self,
        device_id: &str,
        state: LogicalPadStateDto,
    ) -> Result<(), OhMyGamepadServiceError> {
        let descriptor = {
            let mut service_state = self.state.lock().expect("lock service state");
            service_state
                .virtual_descriptors
                .entry(device_id.to_owned())
                .or_insert_with(|| SimulatedGamepadDescriptor {
                    device_id: device_id.to_owned(),
                    name: format!("Simulated Gamepad {device_id}"),
                    ..SimulatedGamepadDescriptor::default()
                })
                .clone()
        };

        self.submit_descriptor_state(&descriptor, state)
    }

    pub fn submit_keyboard_state(
        &self,
        state: LogicalPadStateDto,
    ) -> Result<(), OhMyGamepadServiceError> {
        self.submit_descriptor_state(
            &SimulatedGamepadDescriptor {
                device_id: KEYBOARD_FALLBACK_DEVICE_ID.to_owned(),
                name: KEYBOARD_FALLBACK_DEVICE_NAME.to_owned(),
                ..SimulatedGamepadDescriptor::default()
            },
            state,
        )
    }

    pub fn replace_keyboard_mapping(
        &self,
        mapping: OhMyGamepadKeyboardMappingDto,
    ) -> Result<(), OhMyGamepadServiceError> {
        let mapping = normalize_keyboard_mapping(mapping);
        let send_result = {
            let mut state = self.state.lock().expect("lock service state");
            if let Some(config) = state.desktop_keyboard_config.as_mut() {
                config.mapping = mapping.clone();
            }
            state
                .keyboard_listener
                .as_ref()
                .map(|listener| listener.replace_mapping(mapping))
        };
        if let Some(Err(_)) = send_result {
            if !self.ensure_keyboard_listener_running("replace-keyboard-mapping") {
                return Err(OhMyGamepadServiceError::InputChannelClosed);
            }
        }
        Ok(())
    }

    pub fn play_rumble(
        &self,
        request: OhMyGamepadRumbleRequestDto,
    ) -> Result<OhMyGamepadRumbleResultDto, InputRuntimeError> {
        let snapshot = self.snapshot_with_strategy_sync()?;
        let state = self.state.lock().expect("lock service state");
        let prepared = prepare_rumble_dispatch(
            resolve_connected_target_devices(&snapshot, &request.target, EMPTY_SAMPLING_DEVICE_ID),
            state.rumble_backend.as_deref(),
        );

        match prepared {
            PreparedRumbleRequest::Rejected(result) => Ok(result),
            PreparedRumbleRequest::Dispatch(dispatch) => {
                let rumble_backend = state
                    .rumble_backend
                    .as_ref()
                    .expect("prepared dispatch requires rumble backend");
                rumble_backend.play_rumble(dispatch.device_ids(), &request.effect)?;
                Ok(dispatch.into_result())
            }
        }
    }

    pub fn stop_rumble(
        &self,
        target: OhMyGamepadRumbleTargetDto,
    ) -> Result<OhMyGamepadRumbleResultDto, InputRuntimeError> {
        let snapshot = self.snapshot_with_strategy_sync()?;
        let state = self.state.lock().expect("lock service state");
        let prepared = prepare_rumble_dispatch(
            resolve_connected_target_devices(&snapshot, &target, EMPTY_SAMPLING_DEVICE_ID),
            state.rumble_backend.as_deref(),
        );

        match prepared {
            PreparedRumbleRequest::Rejected(result) => Ok(result),
            PreparedRumbleRequest::Dispatch(dispatch) => {
                let rumble_backend = state
                    .rumble_backend
                    .as_ref()
                    .expect("prepared dispatch requires rumble backend");
                rumble_backend.stop_rumble(dispatch.device_ids())?;
                Ok(dispatch.into_result())
            }
        }
    }

    pub fn apply_command(
        &self,
        command: OhMyGamepadServiceCommandDto,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, OhMyGamepadServiceError> {
        match command {
            OhMyGamepadServiceCommandDto::RefreshSnapshot => {}
            OhMyGamepadServiceCommandDto::SetStreamPadForwarding { enabled } => {
                self.set_stream_pad_forwarding(enabled);
            }
            OhMyGamepadServiceCommandDto::UpdateSampling { sampling } => {
                self.set_sampling(sampling)?;
            }
            OhMyGamepadServiceCommandDto::SetSamplingPreset { preset } => {
                self.set_sampling_preset(preset)?;
            }
            OhMyGamepadServiceCommandDto::SetSamplingStrategy { strategy } => {
                self.set_sampling_strategy(strategy)?;
            }
            OhMyGamepadServiceCommandDto::SetPrimarySamplingDevice { device_id } => {
                self.set_primary_sampling_device(device_id)?;
            }
            OhMyGamepadServiceCommandDto::PauseSamplingDevice { device_id } => {
                self.pause_sampling_device(&device_id)?;
            }
            OhMyGamepadServiceCommandDto::ResumeSamplingDevice { device_id } => {
                self.resume_sampling_device(&device_id)?;
            }
            OhMyGamepadServiceCommandDto::ConnectSimulatedGamepad { descriptor } => {
                self.connect_simulated_gamepad(descriptor)?;
            }
            OhMyGamepadServiceCommandDto::DisconnectSimulatedGamepad { device_id } => {
                self.disconnect_simulated_gamepad(&device_id)?;
            }
            OhMyGamepadServiceCommandDto::SubmitSimulatedState { device_id, state } => {
                self.submit_simulated_state(&device_id, state)?;
            }
            OhMyGamepadServiceCommandDto::SubmitKeyboardState { state } => {
                self.submit_keyboard_state(state)?;
            }
            OhMyGamepadServiceCommandDto::ReplaceKeyboardMapping { mapping } => {
                self.replace_keyboard_mapping(mapping)?;
            }
        }

        self.snapshot_with_strategy_sync()
            .map_err(OhMyGamepadServiceError::from)
    }

    pub fn shutdown(self) -> Result<(), InputRuntimeError> {
        if let Some(listener) = self
            .state
            .lock()
            .expect("lock service state")
            .keyboard_listener
            .take()
        {
            listener.shutdown();
        }
        self.runtime.shutdown()
    }

    fn submit_descriptor_state(
        &self,
        descriptor: &SimulatedGamepadDescriptor,
        state: LogicalPadStateDto,
    ) -> Result<(), OhMyGamepadServiceError> {
        let tx = self
            .state
            .lock()
            .expect("lock service state")
            .virtual_input_tx
            .clone();
        let observed_at_ms = now_ms();
        let mut events = vec![simulated_connection_event(descriptor, true)];
        events.extend(logical_state_to_events(descriptor, observed_at_ms, state));
        send_virtual_events(&tx, events)?;
        if self.should_wait_for_device_state(&descriptor.device_id)? {
            let _ = self.wait_for_device_state(&descriptor.device_id, &state)?;
        }
        Ok(())
    }

    fn snapshot_with_strategy_sync(
        &self,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, InputRuntimeError> {
        let snapshot = self.runtime.get_runtime_snapshot()?;
        if self.processing_mode == InputProcessingMode::StandardDevices {
            return Ok(snapshot);
        }
        if self.reconcile_sampling_binding(&snapshot)? {
            self.runtime.get_runtime_snapshot()
        } else {
            Ok(snapshot)
        }
    }

    fn wait_for_device_presence(
        &self,
        device_id: &str,
        should_exist: bool,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, InputRuntimeError> {
        let mut last_snapshot = self.snapshot_with_strategy_sync()?;
        for _ in 0..SERVICE_SYNC_RETRY_COUNT {
            let exists = last_snapshot
                .devices
                .iter()
                .any(|device| device.device_id == device_id);
            if exists == should_exist {
                return Ok(last_snapshot);
            }

            thread::sleep(std::time::Duration::from_millis(
                SERVICE_SYNC_RETRY_DELAY_MS,
            ));
            last_snapshot = self.snapshot_with_strategy_sync()?;
        }

        Ok(last_snapshot)
    }

    fn wait_for_device_state(
        &self,
        device_id: &str,
        expected_state: &LogicalPadStateDto,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, InputRuntimeError> {
        let mut last_snapshot = self.snapshot_with_strategy_sync()?;
        for _ in 0..SERVICE_SYNC_RETRY_COUNT {
            if last_snapshot.slots.iter().any(|pad| {
                pad.device_ids.iter().any(|id| id == device_id)
                    && logical_state_contains(&pad.state, expected_state)
            }) {
                return Ok(last_snapshot);
            }

            thread::sleep(std::time::Duration::from_millis(
                SERVICE_SYNC_RETRY_DELAY_MS,
            ));
            last_snapshot = self.snapshot_with_strategy_sync()?;
        }

        Ok(last_snapshot)
    }

    fn should_wait_for_device_state(&self, device_id: &str) -> Result<bool, InputRuntimeError> {
        if device_id != KEYBOARD_FALLBACK_DEVICE_ID {
            return Ok(true);
        }

        Ok(self
            .snapshot_with_strategy_sync()?
            .devices
            .iter()
            .any(|device| device.device_id == device_id && device.connected))
    }

    fn reconcile_sampling_binding(
        &self,
        snapshot: &OhMyGamepadRuntimeSnapshotDto,
    ) -> Result<bool, InputRuntimeError> {
        let strategy = self
            .state
            .lock()
            .expect("lock service state")
            .sampling_strategy
            .clone();
        let desired = build_binding(snapshot, &strategy);
        let current = snapshot.slot_bindings.first().cloned().unwrap_or_default();
        if current == desired {
            return Ok(false);
        }

        self.runtime.rebind_logical_pad(desired)?;
        Ok(true)
    }
}

fn normalize_core_config(
    mut core: InputCoreConfig,
    strategy: &MultiControllerSamplingStrategy,
) -> InputCoreConfig {
    if core.processing_mode == InputProcessingMode::StandardDevices {
        return core;
    }
    if core.bindings.is_empty() {
        core.bindings = vec![match strategy.mode {
            MultiControllerSamplingMode::Merge => LogicalPadBindingDto {
                slot: LogicalPadId::Pad0,
                mode: OhMyGamepadBindingModeDto::LastActiveFailover,
                device_ids: Vec::new(),
            },
            MultiControllerSamplingMode::PrimaryPreferred => LogicalPadBindingDto {
                slot: LogicalPadId::Pad0,
                mode: OhMyGamepadBindingModeDto::FixedDevice,
                device_ids: strategy
                    .primary_device_id
                    .clone()
                    .into_iter()
                    .collect::<Vec<_>>(),
            },
        }];
    }
    core
}

fn build_binding(
    snapshot: &OhMyGamepadRuntimeSnapshotDto,
    strategy: &MultiControllerSamplingStrategy,
) -> LogicalPadBindingDto {
    let paused = strategy
        .paused_device_ids
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let connected_ids = snapshot
        .devices
        .iter()
        .filter(|device| device.connected)
        .map(|device| device.device_id.clone())
        .filter(|device_id| !paused.contains(device_id))
        .collect::<Vec<_>>();

    match strategy.mode {
        MultiControllerSamplingMode::Merge => LogicalPadBindingDto {
            slot: LogicalPadId::Pad0,
            mode: OhMyGamepadBindingModeDto::LastActiveFailover,
            device_ids: non_empty_device_ids(connected_ids),
        },
        MultiControllerSamplingMode::PrimaryPreferred => {
            if let Some(primary_device_id) = strategy.primary_device_id.as_ref() {
                if connected_ids
                    .iter()
                    .any(|device_id| device_id == primary_device_id)
                {
                    return LogicalPadBindingDto {
                        slot: LogicalPadId::Pad0,
                        mode: OhMyGamepadBindingModeDto::FixedDevice,
                        device_ids: vec![primary_device_id.clone()],
                    };
                }
            }

            LogicalPadBindingDto {
                slot: LogicalPadId::Pad0,
                mode: OhMyGamepadBindingModeDto::Merged,
                device_ids: non_empty_device_ids(connected_ids),
            }
        }
    }
}

fn non_empty_device_ids(device_ids: Vec<String>) -> Vec<String> {
    if device_ids.is_empty() {
        vec![EMPTY_SAMPLING_DEVICE_ID.to_owned()]
    } else {
        device_ids
    }
}

fn send_virtual_events(
    tx: &Sender<Vec<Sdl3InputEvent>>,
    events: Vec<Sdl3InputEvent>,
) -> Result<(), OhMyGamepadServiceError> {
    tx.send(events)
        .map_err(|_| OhMyGamepadServiceError::InputChannelClosed)
}

fn simulated_connection_event(
    descriptor: &SimulatedGamepadDescriptor,
    connected: bool,
) -> Sdl3InputEvent {
    Sdl3InputEvent {
        device: simulated_descriptor(descriptor),
        observed_at_ms: now_ms(),
        kind: if connected {
            Sdl3InputEventKind::Connected
        } else {
            Sdl3InputEventKind::Disconnected
        },
    }
}

fn simulated_descriptor(descriptor: &SimulatedGamepadDescriptor) -> Sdl3DeviceDescriptor {
    Sdl3DeviceDescriptor {
        device_id: descriptor.device_id.clone(),
        name: descriptor.name.clone(),
        vendor_id: descriptor.vendor_id,
        product_id: descriptor.product_id,
        path: Some(format!("virtual://{}", descriptor.device_id)),
        ..Sdl3DeviceDescriptor::default()
    }
}

fn logical_state_to_events(
    descriptor: &SimulatedGamepadDescriptor,
    observed_at_ms: u64,
    state: LogicalPadStateDto,
) -> Vec<Sdl3InputEvent> {
    // 单源模型下一个逻辑态只产出这一条全量快照。
    vec![state_snapshot_event(
        simulated_descriptor(descriptor),
        observed_at_ms,
        &state,
    )]
}

fn logical_state_contains(actual: &LogicalPadStateDto, expected: &LogicalPadStateDto) -> bool {
    contains_scalar(actual.buttons.south, expected.buttons.south)
        && contains_scalar(actual.buttons.east, expected.buttons.east)
        && contains_scalar(actual.buttons.west, expected.buttons.west)
        && contains_scalar(actual.buttons.north, expected.buttons.north)
        && contains_scalar(actual.buttons.l1, expected.buttons.l1)
        && contains_scalar(actual.buttons.r1, expected.buttons.r1)
        && contains_scalar(actual.buttons.l2, expected.buttons.l2)
        && contains_scalar(actual.buttons.r2, expected.buttons.r2)
        && contains_scalar(actual.buttons.l3, expected.buttons.l3)
        && contains_scalar(actual.buttons.r3, expected.buttons.r3)
        && contains_scalar(actual.buttons.view, expected.buttons.view)
        && contains_scalar(actual.buttons.menu, expected.buttons.menu)
        && contains_scalar(actual.buttons.home, expected.buttons.home)
        && contains_scalar(actual.buttons.dpad_up, expected.buttons.dpad_up)
        && contains_scalar(actual.buttons.dpad_down, expected.buttons.dpad_down)
        && contains_scalar(actual.buttons.dpad_left, expected.buttons.dpad_left)
        && contains_scalar(actual.buttons.dpad_right, expected.buttons.dpad_right)
        && contains_axis(actual.left_stick.x, expected.left_stick.x)
        && contains_axis(actual.left_stick.y, expected.left_stick.y)
        && contains_axis(actual.right_stick.x, expected.right_stick.x)
        && contains_axis(actual.right_stick.y, expected.right_stick.y)
        && contains_scalar(actual.left_trigger, expected.left_trigger)
        && contains_scalar(actual.right_trigger, expected.right_trigger)
}

fn contains_scalar(actual: f32, expected: f32) -> bool {
    if expected.abs() <= f32::EPSILON {
        true
    } else {
        actual + 0.0001 >= expected
    }
}

fn contains_axis(actual: f32, expected: f32) -> bool {
    if expected.abs() <= f32::EPSILON {
        true
    } else if expected.is_sign_positive() {
        actual + 0.0001 >= expected
    } else {
        actual - 0.0001 <= expected
    }
}

fn normalize_keyboard_mapping(
    mapping: OhMyGamepadKeyboardMappingDto,
) -> OhMyGamepadKeyboardMappingDto {
    if mapping.bindings.is_empty() {
        OhMyGamepadKeyboardMappingDto::default()
    } else {
        mapping
    }
}

fn now_ms() -> u64 {
    ohmygamepad_core::monotonic_now_ms()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NoopSdl3Source;

    #[test]
    fn standard_device_service_keeps_sampling_strategy_out_of_the_desktop_path() {
        let mut config = OhMyGamepadServiceConfig::default();
        config.core.processing_mode = InputProcessingMode::StandardDevices;
        config.desktop_keyboard = None;
        config.sampling_strategy.enable_keyboard_fallback = false;
        config.sampling_strategy.primary_device_id = Some("old-primary".into());
        let service = OhMyGamepadService::spawn_with_source(config, NoopSdl3Source);
        let initial = service.snapshot().unwrap();
        assert!(initial.slots.is_empty());
        assert!(initial.slot_bindings.is_empty());
        assert!(initial.devices.is_empty());
        service
            .set_primary_sampling_device(Some("new-primary".into()))
            .unwrap();
        assert!(service.snapshot().unwrap().slot_bindings.is_empty());
        service.shutdown().unwrap();
    }

    #[test]
    fn empty_keyboard_mapping_resets_to_default_mapping() {
        let service = OhMyGamepadService::spawn_with_source(
            OhMyGamepadServiceConfig::default(),
            NoopSdl3Source,
        );

        service
            .replace_keyboard_mapping(OhMyGamepadKeyboardMappingDto {
                bindings: Vec::new(),
            })
            .expect("replace keyboard mapping");

        let bindings_len = service
            .state
            .lock()
            .expect("lock service state")
            .desktop_keyboard_config
            .as_ref()
            .expect("desktop keyboard config")
            .mapping
            .bindings
            .len();
        assert_eq!(
            bindings_len,
            OhMyGamepadKeyboardMappingDto::default().bindings.len()
        );
        service.shutdown().expect("shutdown service");
    }
}
