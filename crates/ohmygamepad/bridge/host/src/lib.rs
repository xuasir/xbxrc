use std::sync::{
    mpsc::{self, Receiver, Sender},
    Arc, Mutex, OnceLock,
};

use ohmygamepad_core::{
    ArcSwap, DesktopDriverSelector, DesktopHapticsProviderKind, DeviceProfile, InputRuntimeError,
};
use ohmygamepad_protocol::{
    LogicalPadBindingDto, LogicalPadSnapshotDto, LogicalPadStateDto,
    MultiControllerSamplingStrategyDto, OhMyGamepadHapticsProviderKindDto,
    OhMyGamepadInputRouteDto, OhMyGamepadKeyboardMappingDto, OhMyGamepadRumbleRequestDto,
    OhMyGamepadRumbleResultDto, OhMyGamepadRumbleTargetDto, OhMyGamepadRuntimeHapticsDto,
    OhMyGamepadRuntimeSnapshotDto, OhMyGamepadSamplingConfigDto,
};
use ohmygamepad_sdl3::{OhMyGamepadService, OhMyGamepadServiceConfig, ShellWindowRouteHints};
#[cfg(target_os = "windows")]
use ohmygamepad_win_xbox_haptics::WindowsXboxHapticsProvider;

static SHARED_GAMEPAD_RUNTIME: OnceLock<Result<SharedGamepadRuntime, String>> = OnceLock::new();

struct SharedGamepadRuntime {
    runtime: Arc<OhMyGamepadService>,
    haptics_provider: DesktopHapticsProviderKind,
    snapshot_broadcaster: Arc<HostRuntimeSnapshotBroadcaster>,
}

#[derive(Clone)]
pub struct GamepadRuntimeHost {
    runtime: Arc<OhMyGamepadService>,
    haptics_provider: DesktopHapticsProviderKind,
    snapshot_broadcaster: Arc<HostRuntimeSnapshotBroadcaster>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GamepadRuntimeHostError {
    message: String,
}

impl GamepadRuntimeHostError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for GamepadRuntimeHostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for GamepadRuntimeHostError {}

impl GamepadRuntimeHost {
    pub fn shared() -> Result<Self, GamepadRuntimeHostError> {
        let runtime = SHARED_GAMEPAD_RUNTIME
            .get_or_init(|| {
                bootstrap_gamepad_runtime()
                    .map_err(|error| format!("bootstrapOhMyGamepadHost:{error}"))
            })
            .as_ref()
            .map_err(|message| GamepadRuntimeHostError::new(message.clone()))?;

        Ok(Self {
            runtime: Arc::clone(&runtime.runtime),
            haptics_provider: runtime.haptics_provider,
            snapshot_broadcaster: Arc::clone(&runtime.snapshot_broadcaster),
        })
    }

    pub fn snapshot(&self) -> Result<OhMyGamepadRuntimeSnapshotDto, InputRuntimeError> {
        self.runtime
            .snapshot()
            .map(|snapshot| enrich_runtime_snapshot(snapshot, self.haptics_provider, &self.runtime))
    }

    /// 无锁读取最新 pad 状态的原子单元；串流输入环等高频读方直接 load，
    /// 避免对 runtime 线程的阻塞往返。
    pub fn latest_pad_states(&self) -> Arc<ArcSwap<Vec<LogicalPadSnapshotDto>>> {
        self.runtime.get_latest_pad_states()
    }

    pub fn subscribe_runtime_snapshot(&self) -> Receiver<OhMyGamepadRuntimeSnapshotDto> {
        self.snapshot_broadcaster.subscribe()
    }

    pub fn set_stream_pad_forwarding(&self, enabled: bool) -> Result<(), InputRuntimeError> {
        self.runtime.set_stream_pad_forwarding(enabled);
        // Forwarding lives outside the runtime snapshot state machine; force a refresh so
        // snapshot subscribers observe the new routing state immediately.
        self.runtime.refresh_snapshot()?;
        self.publish_current_snapshot()?;
        Ok(())
    }

    pub fn stream_pad_forwarding(&self) -> bool {
        self.runtime.stream_pad_forwarding()
    }

    pub fn set_shell_window_route_hints(&self, hints: ShellWindowRouteHints) {
        self.runtime.set_shell_window_route_hints(hints);
        let _ = self.publish_current_snapshot();
    }

    /// 无锁读取最近一次派生的三态输入路由（Phase 4 真相源）。
    pub fn peek_input_route(&self) -> OhMyGamepadInputRouteDto {
        self.runtime.peek_input_route()
    }

    /// 前端 overlay / 菜单 / 诊断页等上报"正在捕获输入"：route 切回 Ui。
    pub fn set_ui_capture(&self, capturing: bool) {
        self.runtime.set_ui_capture(capturing);
        let _ = self.publish_current_snapshot();
    }

    /// 手动重新枚举手柄：仅此入口允许重开设备句柄。
    pub fn rescan_devices(&self) -> Result<OhMyGamepadRuntimeSnapshotDto, InputRuntimeError> {
        self.runtime.rescan_devices()?;
        self.snapshot()
    }

    pub fn set_sampling(
        &self,
        sampling: OhMyGamepadSamplingConfigDto,
    ) -> Result<(), InputRuntimeError> {
        self.runtime.set_sampling(sampling)
    }

    pub fn rebind_logical_pad(
        &self,
        binding: LogicalPadBindingDto,
    ) -> Result<(), InputRuntimeError> {
        self.runtime.rebind_logical_pad(binding)
    }

    pub fn set_sampling_strategy(
        &self,
        strategy: MultiControllerSamplingStrategyDto,
    ) -> Result<(), InputRuntimeError> {
        self.runtime.set_sampling_strategy(strategy)
    }

    fn publish_current_snapshot(&self) -> Result<(), InputRuntimeError> {
        let snapshot = self.snapshot()?;
        self.snapshot_broadcaster.publish(snapshot);
        Ok(())
    }

    pub fn set_primary_sampling_device(
        &self,
        device_id: Option<String>,
    ) -> Result<(), InputRuntimeError> {
        self.runtime.set_primary_sampling_device(device_id)
    }

    pub fn pause_sampling_device(&self, device_id: &str) -> Result<(), InputRuntimeError> {
        self.runtime.pause_sampling_device(device_id)
    }

    pub fn resume_sampling_device(&self, device_id: &str) -> Result<(), InputRuntimeError> {
        self.runtime.resume_sampling_device(device_id)
    }

    pub fn play_rumble(
        &self,
        request: OhMyGamepadRumbleRequestDto,
    ) -> Result<OhMyGamepadRumbleResultDto, InputRuntimeError> {
        self.runtime.play_rumble(request)
    }

    pub fn stop_rumble(
        &self,
        target: OhMyGamepadRumbleTargetDto,
    ) -> Result<OhMyGamepadRumbleResultDto, InputRuntimeError> {
        self.runtime.stop_rumble(target)
    }

    pub fn submit_simulated_state(
        &self,
        device_id: &str,
        state: LogicalPadStateDto,
    ) -> Result<(), GamepadRuntimeHostError> {
        self.runtime
            .submit_simulated_state(device_id, state)
            .map_err(|error| {
                GamepadRuntimeHostError::new(format!("submitSimulatedState:{error:?}"))
            })
    }

    pub fn submit_keyboard_state(
        &self,
        state: LogicalPadStateDto,
    ) -> Result<(), GamepadRuntimeHostError> {
        self.runtime
            .submit_keyboard_state(state)
            .map_err(|error| GamepadRuntimeHostError::new(format!("submitKeyboardState:{error:?}")))
    }

    pub fn replace_device_profiles(
        &self,
        profiles: Vec<DeviceProfile>,
    ) -> Result<(), InputRuntimeError> {
        self.runtime.replace_device_profiles(profiles)
    }

    pub fn replace_keyboard_mapping(
        &self,
        mapping: OhMyGamepadKeyboardMappingDto,
    ) -> Result<(), InputRuntimeError> {
        self.runtime
            .replace_keyboard_mapping(mapping)
            .map_err(|_| InputRuntimeError::CommandChannelClosed)
    }
}

pub fn set_runtime_trace_sink(sink: Option<Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>>) {
    ohmygamepad_sdl3::set_runtime_trace_sink(sink);
}

fn bootstrap_gamepad_runtime() -> Result<SharedGamepadRuntime, String> {
    let config = OhMyGamepadServiceConfig::default();
    let selected_providers = DesktopDriverSelector::select(&config.core);

    let haptics_provider = selected_providers.haptics_provider;
    let runtime = Arc::new(spawn_host_service(config, haptics_provider)?);
    let snapshot_broadcaster = Arc::new(HostRuntimeSnapshotBroadcaster::default());
    let source_rx = runtime.subscribe_runtime_snapshot();
    let publish_runtime = Arc::clone(&runtime);
    let publish_broadcaster = Arc::clone(&snapshot_broadcaster);
    std::thread::spawn(move || {
        while let Ok(snapshot) = source_rx.recv() {
            publish_broadcaster.publish(enrich_runtime_snapshot(
                snapshot,
                haptics_provider,
                &publish_runtime,
            ));
        }
    });

    Ok(SharedGamepadRuntime {
        runtime,
        haptics_provider,
        snapshot_broadcaster,
    })
}

fn enrich_runtime_snapshot(
    mut snapshot: OhMyGamepadRuntimeSnapshotDto,
    haptics_provider: DesktopHapticsProviderKind,
    service: &ohmygamepad_sdl3::OhMyGamepadService,
) -> OhMyGamepadRuntimeSnapshotDto {
    let default_device_id = resolve_default_device_id(&snapshot);

    for device in &mut snapshot.devices {
        device.name = normalize_device_name(device, haptics_provider);
        apply_host_haptics_compat(device, haptics_provider);
    }

    snapshot.haptics = OhMyGamepadRuntimeHapticsDto {
        provider: map_haptics_provider_kind(haptics_provider),
        supports_basic_rumble: snapshot.devices.iter().any(supports_host_basic_rumble),
        supports_trigger_rumble: snapshot.devices.iter().any(supports_host_trigger_rumble),
        default_device_id,
    };
    snapshot.stream_pad_forwarding = service.stream_pad_forwarding();
    // Phase 4 真相源：统一输入路由（取代旧 input_gate）
    snapshot.input_route = service.peek_input_route();
    snapshot
}

fn resolve_default_device_id(snapshot: &OhMyGamepadRuntimeSnapshotDto) -> Option<String> {
    snapshot
        .slots
        .iter()
        .find_map(|pad| pad.device_ids.first().cloned())
        .or_else(|| {
            snapshot
                .devices
                .iter()
                .find(|device| device.connected)
                .map(|device| device.device_id.clone())
        })
}

#[derive(Default)]
struct HostRuntimeSnapshotBroadcaster {
    state: Mutex<HostRuntimeSnapshotBroadcasterState>,
}

#[derive(Default)]
struct HostRuntimeSnapshotBroadcasterState {
    current_snapshot: Option<OhMyGamepadRuntimeSnapshotDto>,
    subscribers: Vec<Sender<OhMyGamepadRuntimeSnapshotDto>>,
}

impl HostRuntimeSnapshotBroadcaster {
    fn subscribe(&self) -> Receiver<OhMyGamepadRuntimeSnapshotDto> {
        let (tx, rx) = mpsc::channel();
        let current_snapshot = {
            let mut state = self
                .state
                .lock()
                .expect("lock host runtime snapshot broadcaster");
            state.subscribers.push(tx.clone());
            state.current_snapshot.clone()
        };
        if let Some(snapshot) = current_snapshot {
            let _ = tx.send(snapshot);
        }
        rx
    }

    fn publish(&self, snapshot: OhMyGamepadRuntimeSnapshotDto) {
        let mut state = self
            .state
            .lock()
            .expect("lock host runtime snapshot broadcaster");
        if state.current_snapshot.as_ref() == Some(&snapshot) {
            return;
        }
        state.current_snapshot = Some(snapshot.clone());
        state
            .subscribers
            .retain(|subscriber| subscriber.send(snapshot.clone()).is_ok());
    }
}

fn normalize_device_name(
    device: &ohmygamepad_protocol::OhMyGamepadDeviceDto,
    haptics_provider: DesktopHapticsProviderKind,
) -> String {
    let trimmed = device.name.trim();
    if trimmed.is_empty() {
        return "Controller".to_owned();
    }

    if should_force_xbox_label(device, haptics_provider) {
        return "Xbox Controller".to_owned();
    }

    trimmed.to_owned()
}

fn should_force_xbox_label(
    device: &ohmygamepad_protocol::OhMyGamepadDeviceDto,
    haptics_provider: DesktopHapticsProviderKind,
) -> bool {
    haptics_provider == DesktopHapticsProviderKind::WinXboxHaptics
        && is_rog_xbox_ally_x_xinput_view(device)
}

fn map_haptics_provider_kind(
    provider: DesktopHapticsProviderKind,
) -> OhMyGamepadHapticsProviderKindDto {
    match provider {
        DesktopHapticsProviderKind::Sdl3Gamepad => OhMyGamepadHapticsProviderKindDto::Sdl3Gamepad,
        DesktopHapticsProviderKind::WinXboxHaptics => {
            OhMyGamepadHapticsProviderKindDto::WinXboxHaptics
        }
    }
}

fn spawn_host_service(
    config: OhMyGamepadServiceConfig,
    haptics_provider: DesktopHapticsProviderKind,
) -> Result<OhMyGamepadService, String> {
    match haptics_provider {
        DesktopHapticsProviderKind::Sdl3Gamepad => {
            OhMyGamepadService::spawn(config).map_err(|error| error.to_string())
        }
        DesktopHapticsProviderKind::WinXboxHaptics => spawn_windows_xbox_haptics_service(config),
    }
}

#[cfg(target_os = "windows")]
fn spawn_windows_xbox_haptics_service(
    config: OhMyGamepadServiceConfig,
) -> Result<OhMyGamepadService, String> {
    OhMyGamepadService::spawn_with_haptics_provider(config, Box::new(WindowsXboxHapticsProvider))
        .map_err(|error| error.to_string())
}

#[cfg(not(target_os = "windows"))]
fn spawn_windows_xbox_haptics_service(
    config: OhMyGamepadServiceConfig,
) -> Result<OhMyGamepadService, String> {
    OhMyGamepadService::spawn(config).map_err(|error| error.to_string())
}

fn apply_host_haptics_compat(
    device: &mut ohmygamepad_protocol::OhMyGamepadDeviceDto,
    haptics_provider: DesktopHapticsProviderKind,
) {
    if haptics_provider == DesktopHapticsProviderKind::WinXboxHaptics
        && is_rog_xbox_ally_x_xinput_view(device)
    {
        device.sdl3_capabilities.supports_trigger_rumble = true;
    }
}

fn supports_host_basic_rumble(device: &ohmygamepad_protocol::OhMyGamepadDeviceDto) -> bool {
    device.sdl3_capabilities.supports_rumble || supports_host_trigger_rumble(device)
}

fn supports_host_trigger_rumble(device: &ohmygamepad_protocol::OhMyGamepadDeviceDto) -> bool {
    device.sdl3_capabilities.supports_trigger_rumble
}

fn is_rog_xbox_ally_x_xinput_view(device: &ohmygamepad_protocol::OhMyGamepadDeviceDto) -> bool {
    let vendor_match = device.vendor_id == Some(0x0b05);
    let product_match = device.product_id == Some(0x1b4c);
    let lower_name = device.name.to_ascii_lowercase();
    let lower_path = device
        .path
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let lower_mapping = device
        .mapping
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();

    vendor_match
        && product_match
        && (lower_name.contains("xinput")
            || lower_path.contains("xinput")
            || lower_mapping.starts_with("xinput"))
}

#[cfg(test)]
mod tests {
    use super::HostRuntimeSnapshotBroadcaster;
    use ohmygamepad_protocol::{OhMyGamepadInputRouteDto, OhMyGamepadRuntimeSnapshotDto};

    #[test]
    fn host_runtime_snapshot_broadcaster_publishes_route_only_changes() {
        let broadcaster = HostRuntimeSnapshotBroadcaster::default();
        let rx = broadcaster.subscribe();

        let mut snapshot = OhMyGamepadRuntimeSnapshotDto::default();
        snapshot.input_route = OhMyGamepadInputRouteDto::Blocked;
        broadcaster.publish(snapshot.clone());

        let first = rx.recv().expect("receive first snapshot");
        assert_eq!(first.input_route, OhMyGamepadInputRouteDto::Blocked);

        snapshot.input_route = OhMyGamepadInputRouteDto::Stream;
        broadcaster.publish(snapshot.clone());

        let second = rx.recv().expect("receive second snapshot");
        assert_eq!(second.input_route, OhMyGamepadInputRouteDto::Stream);
    }
}
