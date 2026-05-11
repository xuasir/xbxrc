use std::{
    collections::{HashMap, HashSet},
    sync::{
        mpsc::{self, Receiver, Sender},
        Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use ohmygamepad_core::{
    DeviceProfile, HapticsProvider, InputCoreConfig, InputRuntimeError, InputRuntimeHandle,
    NoopStreamSink, NoopUiSink,
};
use ohmygamepad_protocol::{
    LogicalPadBindingDto, LogicalPadId, LogicalPadStateDto, MultiControllerSamplingModeDto,
    MultiControllerSamplingStrategyDto, OhMyGamepadBindingModeDto, OhMyGamepadDeviceDto,
    OhMyGamepadInputPolicyDto, OhMyGamepadKeyboardMappingDto, OhMyGamepadRumbleRequestDto,
    OhMyGamepadRumbleResultDto, OhMyGamepadRumbleTargetDto, OhMyGamepadRuntimeSnapshotDto,
    OhMyGamepadSamplingConfigDto, OhMyGamepadSamplingHealthDto, OhMyGamepadSamplingLifecycleDto,
    OhMyGamepadSamplingPresetDto, OhMyGamepadServiceCommandDto, SimulatedGamepadDescriptorDto,
};

use crate::service_keyboard::{
    spawn_keyboard_listener_thread, ServiceKeyboardListenerHandle, KEYBOARD_FALLBACK_DEVICE_ID,
    KEYBOARD_FALLBACK_DEVICE_NAME,
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
    keyboard_listener: Option<ServiceKeyboardListenerHandle>,
    source_handle: Option<crate::Sdl3RumbleHandle>,
    rumble_backend: Option<Box<dyn ServiceRumbleBackend>>,
}

pub struct OhMyGamepadService {
    runtime: InputRuntimeHandle,
    state: Mutex<OhMyGamepadServiceState>,
    last_stalled_self_heal_at: Mutex<Option<Instant>>,
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

        let runtime = spawn_sdl3_input_runtime_with_source(
            normalize_core_config(config.core, &config.sampling_strategy),
            config.backend,
            source,
            NoopUiSink,
            NoopStreamSink,
        );
        let keyboard_listener = if config.sampling_strategy.enable_keyboard_fallback {
            config.desktop_keyboard.map(|keyboard_config| {
                spawn_keyboard_listener_thread(keyboard_config, virtual_input_tx.clone(), now_ms)
            })
        } else {
            None
        };

        Self {
            runtime,
            state: Mutex::new(OhMyGamepadServiceState {
                sampling_strategy: config.sampling_strategy,
                virtual_input_tx,
                virtual_descriptors: HashMap::new(),
                keyboard_listener,
                source_handle,
                rumble_backend,
            }),
            last_stalled_self_heal_at: Mutex::new(None),
        }
    }

    fn prime_and_refresh_runtime_sampling(&self) -> Result<(), InputRuntimeError> {
        if let Some(source_handle) = self
            .state
            .lock()
            .expect("lock service state")
            .source_handle
            .as_ref()
            .cloned()
        {
            if let Err(error) = source_handle.prime_sampling() {
                log::warn!("ohmygamepad_prime_sampling_failed error={}", error);
            }
        }
        self.runtime.refresh_snapshot()
    }

    pub fn snapshot(&self) -> Result<OhMyGamepadRuntimeSnapshotDto, InputRuntimeError> {
        self.snapshot_with_strategy_sync()
    }

    pub fn subscribe_runtime_snapshot(&self) -> Receiver<OhMyGamepadRuntimeSnapshotDto> {
        self.runtime.subscribe_runtime_snapshot()
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

    pub fn set_suspended(&self, suspended: bool) -> Result<(), InputRuntimeError> {
        if suspended {
            log::info!("ohmygamepad_suspend_transition action=suspend");
            self.runtime.set_suspended(true)
        } else {
            let policy = self.snapshot_with_strategy_sync()?.input_policy;
            let _ = self.perform_resume_recovery(policy, "set_suspended(false)")?;
            Ok(())
        }
    }

    pub fn set_sampling_lifecycle(
        &self,
        lifecycle: OhMyGamepadSamplingLifecycleDto,
    ) -> Result<(), InputRuntimeError> {
        self.runtime.set_sampling_lifecycle(lifecycle)
    }

    /// 当快照报告 `stalled` 时尝试 prime + refresh，带冷却；成功执行链后置 `bump_sampling_self_heal_count`。
    pub fn try_stalled_sampling_self_heal(&self) -> Result<bool, InputRuntimeError> {
        let snapshot = self.snapshot_with_strategy_sync()?;
        if snapshot.sampling_health != OhMyGamepadSamplingHealthDto::Stalled {
            return Ok(false);
        }

        const COOLDOWN: Duration = Duration::from_secs(2);
        let now = Instant::now();
        {
            let mut last = self
                .last_stalled_self_heal_at
                .lock()
                .expect("lock stalled self heal");
            if let Some(prev) = *last {
                if now.duration_since(prev) < COOLDOWN {
                    return Ok(false);
                }
            }
            *last = Some(now);
        }

        log::info!("ohmygamepad_stalled_self_heal_attempt");
        self.prime_and_refresh_runtime_sampling()?;
        self.runtime.bump_sampling_self_heal_count()?;
        Ok(true)
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

    pub fn set_input_policy(
        &self,
        policy: OhMyGamepadInputPolicyDto,
    ) -> Result<(), InputRuntimeError> {
        self.runtime.set_input_policy(policy)
    }

    pub fn activate_sampling(
        &self,
        policy: Option<OhMyGamepadInputPolicyDto>,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, InputRuntimeError> {
        let target_policy = match policy {
            Some(policy) => policy,
            None => self.snapshot_with_strategy_sync()?.input_policy,
        };
        self.perform_resume_recovery(target_policy, "activate_sampling")
    }

    pub fn resume_shell_sampling(
        &self,
        policy: OhMyGamepadInputPolicyDto,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, InputRuntimeError> {
        log::info!("ohmygamepad_shell_recovery_start policy={:?}", policy);
        self.runtime
            .set_sampling_lifecycle(OhMyGamepadSamplingLifecycleDto::Active)?;
        self.runtime.set_input_policy(policy)?;
        self.prime_and_refresh_runtime_sampling()?;
        let snapshot = self.snapshot_with_strategy_sync()?;
        log_resume_snapshot("resume_shell_sampling", &snapshot);
        Ok(snapshot)
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
        let state = self.state.lock().expect("lock service state");
        if let Some(listener) = state.keyboard_listener.as_ref() {
            listener
                .replace_mapping(mapping)
                .map_err(|_| OhMyGamepadServiceError::InputChannelClosed)?;
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
            OhMyGamepadServiceCommandDto::SetInputPolicy { policy } => {
                self.set_input_policy(policy)?;
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

    fn perform_resume_recovery(
        &self,
        policy: OhMyGamepadInputPolicyDto,
        reason: &str,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, InputRuntimeError> {
        log::info!(
            "ohmygamepad_resume_recovery_start reason={} policy={:?}",
            reason,
            policy
        );
        // 恢复 API 的语义是重新进入可操作采样态；仅 prime/refresh 不足以让
        // BackgroundWarm 下的 slotSnapshot/input action 重新对外发布。
        self.runtime
            .set_sampling_lifecycle(OhMyGamepadSamplingLifecycleDto::Active)?;
        self.runtime.set_suspended(false)?;
        self.runtime.set_input_policy(policy)?;
        self.prime_and_refresh_runtime_sampling()?;
        let snapshot = self.snapshot_with_strategy_sync()?;
        log_resume_snapshot(reason, &snapshot);
        Ok(snapshot)
    }

    fn snapshot_with_strategy_sync(
        &self,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, InputRuntimeError> {
        let snapshot = self.runtime.get_runtime_snapshot()?;
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
    if core.bindings.is_empty() {
        core.bindings = vec![match strategy.mode {
            MultiControllerSamplingMode::Merge => LogicalPadBindingDto {
                slot: LogicalPadId::Pad0,
                mode: OhMyGamepadBindingModeDto::Merged,
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
        MultiControllerSamplingMode::Merge => {
            if paused.is_empty() {
                LogicalPadBindingDto {
                    slot: LogicalPadId::Pad0,
                    mode: OhMyGamepadBindingModeDto::Merged,
                    device_ids: Vec::new(),
                }
            } else {
                LogicalPadBindingDto {
                    slot: LogicalPadId::Pad0,
                    mode: OhMyGamepadBindingModeDto::Merged,
                    device_ids: non_empty_device_ids(connected_ids),
                }
            }
        }
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
    let device = simulated_descriptor(descriptor);
    let left_trigger_value = state.buttons.l2.max(state.left_trigger);
    let right_trigger_value = state.buttons.r2.max(state.right_trigger);

    let mut events = Vec::new();
    for (index, value) in [
        (0, state.buttons.south),
        (1, state.buttons.east),
        (2, state.buttons.west),
        (3, state.buttons.north),
        (4, state.buttons.l1),
        (5, state.buttons.r1),
        (6, left_trigger_value),
        (7, right_trigger_value),
        (8, state.buttons.view),
        (9, state.buttons.menu),
        (10, state.buttons.l3),
        (11, state.buttons.r3),
        (12, state.buttons.dpad_up),
        (13, state.buttons.dpad_down),
        (14, state.buttons.dpad_left),
        (15, state.buttons.dpad_right),
        (16, state.buttons.home),
    ] {
        events.push(Sdl3InputEvent {
            device: device.clone(),
            observed_at_ms,
            kind: Sdl3InputEventKind::ButtonChanged { index, value },
        });
    }

    for (index, value) in [
        (0, state.left_stick.x),
        (1, state.left_stick.y),
        (2, state.right_stick.x),
        (3, state.right_stick.y),
        (4, trigger_axis_value(state.left_trigger)),
        (5, trigger_axis_value(state.right_trigger)),
    ] {
        events.push(Sdl3InputEvent {
            device: device.clone(),
            observed_at_ms,
            kind: Sdl3InputEventKind::AxisChanged { index, value },
        });
    }

    events
}

fn trigger_axis_value(value: f32) -> f32 {
    (value.clamp(0.0, 1.0) * 2.0) - 1.0
}

fn log_resume_snapshot(reason: &str, snapshot: &OhMyGamepadRuntimeSnapshotDto) {
    let connected_devices = snapshot
        .devices
        .iter()
        .filter(|device| device.connected)
        .map(|device| {
            format!(
                "{}|{}|{:04x}:{:04x}|{}|{}|{}|{}",
                device.device_id,
                device.name,
                device.vendor_id.unwrap_or_default(),
                device.product_id.unwrap_or_default(),
                device.path.as_deref().unwrap_or_default(),
                device
                    .mapping
                    .as_deref()
                    .map(mapping_guid_hint)
                    .unwrap_or_default(),
                device.serial_number.as_deref().unwrap_or_default(),
                device
                    .player_index
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>()
        .join(";");
    log::info!(
        "ohmygamepad_resume_recovery_done reason={} devices={} slots={} input_policy={:?} connected=[{}]",
        reason,
        snapshot.devices.len(),
        snapshot.slots.len(),
        snapshot.input_policy,
        connected_devices,
    );
}

fn mapping_guid_hint(mapping: &str) -> &str {
    mapping.split(',').next().unwrap_or_default()
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

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NoopSdl3Source;
    use ohmygamepad_protocol::{OhMyGamepadInputPolicyDto, OhMyGamepadSamplingLifecycleDto};

    #[test]
    fn resume_shell_sampling_promotes_background_warm_to_active() {
        let service = OhMyGamepadService::spawn_with_source(
            OhMyGamepadServiceConfig::default(),
            NoopSdl3Source,
        );

        service
            .set_sampling_lifecycle(OhMyGamepadSamplingLifecycleDto::BackgroundWarm)
            .expect("set background warm");

        let snapshot = service
            .resume_shell_sampling(OhMyGamepadInputPolicyDto::Shared)
            .expect("resume shell sampling");

        assert_eq!(
            snapshot.sampling_lifecycle,
            OhMyGamepadSamplingLifecycleDto::Active
        );
    }
}
