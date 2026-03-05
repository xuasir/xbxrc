use std::{
    sync::{
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use ohmygamepad_protocol::{
    LogicalPadBindingDto, OhMyGamepadRouteTargetDto, OhMyGamepadRuntimeSnapshotDto,
    OhMyGamepadSamplingConfigDto,
};

use crate::{DeviceProfile, InputBackend, InputCore, InputCoreConfig, StreamSink, UiSink};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SamplingActions {
    poll_backend: bool,
    sample_pads: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SamplingSchedule {
    backend_poll_interval: Duration,
    pad_sample_interval: Duration,
    next_backend_poll_at: Duration,
    next_pad_sample_at: Duration,
}

impl SamplingSchedule {
    pub fn new(sampling: &OhMyGamepadSamplingConfigDto) -> Self {
        Self::with_origin(Duration::ZERO, sampling)
    }

    pub(crate) fn with_origin(now: Duration, sampling: &OhMyGamepadSamplingConfigDto) -> Self {
        let backend_poll_interval = interval_from_hz(sampling.backend_poll_rate_hz);
        let pad_sample_interval = interval_from_hz(sampling.logical_pad_sample_rate_hz);
        Self {
            backend_poll_interval,
            pad_sample_interval,
            // 线程启动后先立即执行一次，先建立设备表和首帧快照。
            next_backend_poll_at: now,
            next_pad_sample_at: now,
        }
    }

    pub(crate) fn update_sampling(
        &mut self,
        now: Duration,
        sampling: &OhMyGamepadSamplingConfigDto,
    ) {
        *self = Self::with_origin(now, sampling);
    }

    pub(crate) fn take_due(&mut self, now: Duration) -> SamplingActions {
        let mut actions = SamplingActions::default();

        if now >= self.next_backend_poll_at {
            actions.poll_backend = true;
            self.next_backend_poll_at =
                advance_deadline(self.next_backend_poll_at, self.backend_poll_interval, now);
        }

        if now >= self.next_pad_sample_at {
            actions.sample_pads = true;
            self.next_pad_sample_at =
                advance_deadline(self.next_pad_sample_at, self.pad_sample_interval, now);
        }

        actions
    }

    pub(crate) fn next_deadline(&self) -> Duration {
        self.next_backend_poll_at.min(self.next_pad_sample_at)
    }
}

#[derive(Debug)]
enum RuntimeCommand {
    GetRuntimeSnapshot {
        reply_tx: Sender<OhMyGamepadRuntimeSnapshotDto>,
    },
    SetRouteTarget {
        target: OhMyGamepadRouteTargetDto,
    },
    UpdateSampling {
        sampling: OhMyGamepadSamplingConfigDto,
    },
    RebindLogicalPad {
        binding: LogicalPadBindingDto,
    },
    ReplaceDeviceProfiles {
        profiles: Vec<DeviceProfile>,
    },
    Shutdown,
}

#[derive(Debug, Eq, PartialEq)]
pub enum InputRuntimeError {
    CommandChannelClosed,
    ResponseChannelClosed,
    ThreadJoinFailed,
}

pub struct InputRuntimeHandle {
    command_tx: Sender<RuntimeCommand>,
    snapshot_broadcaster: Arc<RuntimeSnapshotBroadcaster>,
    join_handle: Option<JoinHandle<()>>,
}

impl InputRuntimeHandle {
    pub fn get_runtime_snapshot(&self) -> Result<OhMyGamepadRuntimeSnapshotDto, InputRuntimeError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.command_tx
            .send(RuntimeCommand::GetRuntimeSnapshot { reply_tx })
            .map_err(|_| InputRuntimeError::CommandChannelClosed)?;
        reply_rx
            .recv()
            .map_err(|_| InputRuntimeError::ResponseChannelClosed)
    }

    pub fn set_route_target(
        &self,
        target: OhMyGamepadRouteTargetDto,
    ) -> Result<(), InputRuntimeError> {
        self.command_tx
            .send(RuntimeCommand::SetRouteTarget { target })
            .map_err(|_| InputRuntimeError::CommandChannelClosed)
    }

    pub fn update_sampling(
        &self,
        sampling: OhMyGamepadSamplingConfigDto,
    ) -> Result<(), InputRuntimeError> {
        self.command_tx
            .send(RuntimeCommand::UpdateSampling { sampling })
            .map_err(|_| InputRuntimeError::CommandChannelClosed)
    }

    pub fn rebind_logical_pad(
        &self,
        binding: LogicalPadBindingDto,
    ) -> Result<(), InputRuntimeError> {
        self.command_tx
            .send(RuntimeCommand::RebindLogicalPad { binding })
            .map_err(|_| InputRuntimeError::CommandChannelClosed)
    }

    pub fn replace_device_profiles(
        &self,
        profiles: Vec<DeviceProfile>,
    ) -> Result<(), InputRuntimeError> {
        self.command_tx
            .send(RuntimeCommand::ReplaceDeviceProfiles { profiles })
            .map_err(|_| InputRuntimeError::CommandChannelClosed)
    }

    pub fn subscribe_runtime_snapshot(&self) -> Receiver<OhMyGamepadRuntimeSnapshotDto> {
        self.snapshot_broadcaster.subscribe()
    }

    pub fn shutdown(mut self) -> Result<(), InputRuntimeError> {
        self.command_tx
            .send(RuntimeCommand::Shutdown)
            .map_err(|_| InputRuntimeError::CommandChannelClosed)?;
        if let Some(join_handle) = self.join_handle.take() {
            join_handle
                .join()
                .map_err(|_| InputRuntimeError::ThreadJoinFailed)?;
        }
        Ok(())
    }
}

pub fn spawn_input_runtime<TBackend, TUiSink, TStreamSink>(
    config: InputCoreConfig,
    backend: TBackend,
    ui_sink: TUiSink,
    stream_sink: TStreamSink,
) -> InputRuntimeHandle
where
    TBackend: InputBackend + Send + 'static,
    TUiSink: UiSink + Send + 'static,
    TStreamSink: StreamSink + Send + 'static,
{
    let (command_tx, command_rx) = mpsc::channel();
    let snapshot_broadcaster = Arc::new(RuntimeSnapshotBroadcaster::default());
    let runtime_snapshot_broadcaster = Arc::clone(&snapshot_broadcaster);
    let join_handle = thread::spawn(move || {
        let mut core = InputCore::new(config, backend, ui_sink, stream_sink);
        let origin = Instant::now();
        let mut schedule = SamplingSchedule::new(&core.config().sampling);
        runtime_snapshot_broadcaster.publish(core.runtime_snapshot());
        run_runtime_loop(
            &mut core,
            &command_rx,
            origin,
            &mut schedule,
            &runtime_snapshot_broadcaster,
        );
    });

    InputRuntimeHandle {
        command_tx,
        snapshot_broadcaster,
        join_handle: Some(join_handle),
    }
}

fn run_runtime_loop<TBackend, TUiSink, TStreamSink>(
    core: &mut InputCore<TBackend, TUiSink, TStreamSink>,
    command_rx: &Receiver<RuntimeCommand>,
    origin: Instant,
    schedule: &mut SamplingSchedule,
    snapshot_broadcaster: &RuntimeSnapshotBroadcaster,
) where
    TBackend: InputBackend,
    TUiSink: UiSink,
    TStreamSink: StreamSink,
{
    loop {
        let now = origin.elapsed();
        if apply_due_actions(core, schedule.take_due(now), snapshot_broadcaster) {
            continue;
        }

        let timeout = schedule
            .next_deadline()
            .checked_sub(origin.elapsed())
            .unwrap_or_default();

        match command_rx.recv_timeout(timeout) {
            Ok(command) => {
                if handle_runtime_command(
                    core,
                    schedule,
                    origin.elapsed(),
                    command,
                    snapshot_broadcaster,
                ) {
                    break;
                }

                while let Ok(command) = command_rx.try_recv() {
                    if handle_runtime_command(
                        core,
                        schedule,
                        origin.elapsed(),
                        command,
                        snapshot_broadcaster,
                    ) {
                        return;
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn apply_due_actions<TBackend, TUiSink, TStreamSink>(
    core: &mut InputCore<TBackend, TUiSink, TStreamSink>,
    actions: SamplingActions,
    snapshot_broadcaster: &RuntimeSnapshotBroadcaster,
) -> bool
where
    TBackend: InputBackend,
    TUiSink: UiSink,
    TStreamSink: StreamSink,
{
    let mut applied = false;
    if actions.poll_backend {
        core.poll_backend();
        applied = true;
    }
    if actions.sample_pads {
        core.sample_once();
        applied = true;
    }
    if applied {
        snapshot_broadcaster.publish(core.runtime_snapshot());
    }
    applied
}

fn handle_runtime_command<TBackend, TUiSink, TStreamSink>(
    core: &mut InputCore<TBackend, TUiSink, TStreamSink>,
    schedule: &mut SamplingSchedule,
    now: Duration,
    command: RuntimeCommand,
    snapshot_broadcaster: &RuntimeSnapshotBroadcaster,
) -> bool
where
    TBackend: InputBackend,
    TUiSink: UiSink,
    TStreamSink: StreamSink,
{
    match command {
        RuntimeCommand::GetRuntimeSnapshot { reply_tx } => {
            let _ = reply_tx.send(core.runtime_snapshot());
            false
        }
        RuntimeCommand::SetRouteTarget { target } => {
            core.replace_route_target(target);
            snapshot_broadcaster.publish(core.runtime_snapshot());
            false
        }
        RuntimeCommand::UpdateSampling { sampling } => {
            core.replace_sampling_config(sampling);
            schedule.update_sampling(now, &core.config().sampling);
            snapshot_broadcaster.publish(core.runtime_snapshot());
            false
        }
        RuntimeCommand::RebindLogicalPad { binding } => {
            let mut bindings = core.config().bindings.clone();
            if let Some(index) = bindings
                .iter()
                .position(|item| item.pad_id == binding.pad_id)
            {
                bindings[index] = binding;
            } else {
                bindings.push(binding);
            }
            core.replace_bindings(bindings);
            snapshot_broadcaster.publish(core.runtime_snapshot());
            false
        }
        RuntimeCommand::ReplaceDeviceProfiles { profiles } => {
            core.replace_device_profiles(profiles);
            snapshot_broadcaster.publish(core.runtime_snapshot());
            false
        }
        RuntimeCommand::Shutdown => true,
    }
}

#[derive(Default)]
struct RuntimeSnapshotBroadcaster {
    state: Mutex<RuntimeSnapshotBroadcasterState>,
}

#[derive(Default)]
struct RuntimeSnapshotBroadcasterState {
    current_snapshot: Option<OhMyGamepadRuntimeSnapshotDto>,
    subscribers: Vec<Sender<OhMyGamepadRuntimeSnapshotDto>>,
}

impl RuntimeSnapshotBroadcaster {
    fn subscribe(&self) -> Receiver<OhMyGamepadRuntimeSnapshotDto> {
        let (tx, rx) = mpsc::channel();
        let current_snapshot = {
            let mut state = self
                .state
                .lock()
                .expect("lock runtime snapshot broadcaster");
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
            .expect("lock runtime snapshot broadcaster");
        if state.current_snapshot.as_ref() == Some(&snapshot) {
            return;
        }

        state.current_snapshot = Some(snapshot.clone());
        state
            .subscribers
            .retain(|subscriber| subscriber.send(snapshot.clone()).is_ok());
    }
}

fn interval_from_hz(hz: u16) -> Duration {
    let normalized_hz = hz.max(1) as f64;
    Duration::from_secs_f64(1.0 / normalized_hz)
}

fn advance_deadline(previous: Duration, interval: Duration, now: Duration) -> Duration {
    let mut next = previous + interval;
    while next <= now {
        next += interval;
    }
    next
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
        thread,
        time::{Duration, Instant},
    };

    use ohmygamepad_protocol::{
        LogicalPadSnapshotDto, OhMyGamepadCapabilityFlagsDto, OhMyGamepadDeviceDto,
        OhMyGamepadRouteTargetDto,
    };

    use super::{spawn_input_runtime, SamplingSchedule};
    use crate::{
        BackendPollResult, ButtonMapping, DeviceLifecycleEvent, DeviceProfileMatcher, FilterConfig,
        InputBackend, InputCoreConfig, RawDeviceSample, StreamSink, UiSink,
    };

    #[derive(Default)]
    struct ScriptedBackend {
        polls: VecDeque<BackendPollResult>,
    }

    impl ScriptedBackend {
        fn new(polls: Vec<BackendPollResult>) -> Self {
            Self {
                polls: VecDeque::from(polls),
            }
        }
    }

    impl InputBackend for ScriptedBackend {
        fn poll(&mut self) -> BackendPollResult {
            self.polls.pop_front().unwrap_or_default()
        }
    }

    #[derive(Default, Clone)]
    struct ThreadSafeUiSink {
        pads: Arc<Mutex<Vec<LogicalPadSnapshotDto>>>,
    }

    impl UiSink for ThreadSafeUiSink {
        fn emit_devices_changed(&mut self, _devices: &[OhMyGamepadDeviceDto]) {}

        fn emit_pad_snapshot(&mut self, snapshot: &LogicalPadSnapshotDto) {
            self.pads.lock().expect("lock pads").push(snapshot.clone());
        }
    }

    #[derive(Default, Clone)]
    struct ThreadSafeStreamSink;

    impl StreamSink for ThreadSafeStreamSink {
        fn emit_pad_snapshot(&mut self, _snapshot: &LogicalPadSnapshotDto) {}
    }

    fn device(device_id: &str) -> OhMyGamepadDeviceDto {
        OhMyGamepadDeviceDto {
            device_id: device_id.to_owned(),
            name: format!("device-{device_id}"),
            backend: None,
            connection: None,
            vendor_id: None,
            product_id: None,
            connected: true,
            last_seen_at_ms: 0,
            capabilities: OhMyGamepadCapabilityFlagsDto {
                basic_rumble: false,
                advanced_haptics: false,
                battery: false,
            },
        }
    }

    fn sample(device_id: &str, observed_at_ms: u64, buttons: Vec<f32>) -> RawDeviceSample {
        RawDeviceSample {
            device_id: device_id.to_owned(),
            observed_at_ms,
            buttons,
            axes: vec![0.0, 0.0, 0.0, 0.0],
        }
    }

    fn wait_until<F>(timeout: Duration, predicate: F) -> bool
    where
        F: Fn() -> bool,
    {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if predicate() {
                return true;
            }
            thread::sleep(Duration::from_millis(2));
        }
        predicate()
    }

    #[test]
    fn sampling_schedule_tracks_backend_and_pad_rates_independently() {
        let mut schedule =
            SamplingSchedule::new(&ohmygamepad_protocol::OhMyGamepadSamplingConfigDto {
                backend_poll_rate_hz: 250,
                logical_pad_sample_rate_hz: 60,
                ui_push_rate_hz: 60,
                stream_push_mode: ohmygamepad_protocol::OhMyGamepadStreamPushModeDto::OnChange,
                stream_push_rate_hz: None,
            });

        let first = schedule.take_due(Duration::ZERO);
        assert!(first.poll_backend);
        assert!(first.sample_pads);

        let second = schedule.take_due(Duration::from_millis(4));
        assert!(second.poll_backend);
        assert!(!second.sample_pads);

        let third = schedule.take_due(Duration::from_millis(17));
        assert!(third.poll_backend);
        assert!(third.sample_pads);
    }

    #[test]
    fn sampling_schedule_reset_takes_effect_immediately() {
        let mut schedule =
            SamplingSchedule::new(&ohmygamepad_protocol::OhMyGamepadSamplingConfigDto {
                backend_poll_rate_hz: 125,
                logical_pad_sample_rate_hz: 125,
                ui_push_rate_hz: 60,
                stream_push_mode: ohmygamepad_protocol::OhMyGamepadStreamPushModeDto::OnChange,
                stream_push_rate_hz: None,
            });
        let _ = schedule.take_due(Duration::ZERO);

        schedule.update_sampling(
            Duration::from_millis(10),
            &ohmygamepad_protocol::OhMyGamepadSamplingConfigDto {
                backend_poll_rate_hz: 500,
                logical_pad_sample_rate_hz: 500,
                ui_push_rate_hz: 60,
                stream_push_mode: ohmygamepad_protocol::OhMyGamepadStreamPushModeDto::OnChange,
                stream_push_rate_hz: None,
            },
        );

        let due = schedule.take_due(Duration::from_millis(10));
        assert!(due.poll_backend);
        assert!(due.sample_pads);
    }

    #[test]
    fn runtime_thread_emits_snapshot_and_accepts_route_update() {
        let ui_sink = ThreadSafeUiSink::default();
        let pads = ui_sink.pads.clone();
        let backend = ScriptedBackend::new(vec![BackendPollResult {
            device_events: vec![DeviceLifecycleEvent::Added(device("pad-a"))],
            samples: vec![sample("pad-a", 10, vec![1.0])],
        }]);
        let runtime = spawn_input_runtime(
            InputCoreConfig::default(),
            backend,
            ui_sink,
            ThreadSafeStreamSink,
        );

        assert!(wait_until(Duration::from_millis(80), || {
            !pads.lock().expect("lock pads").is_empty()
        }));

        let snapshot = runtime
            .get_runtime_snapshot()
            .expect("runtime snapshot should be available");
        assert_eq!(snapshot.pads.len(), 1);
        assert_eq!(snapshot.pads[0].state.buttons.south, 1.0);

        runtime
            .set_route_target(OhMyGamepadRouteTargetDto::StreamSession {
                session_id: "session-1".to_owned(),
            })
            .expect("route target update should succeed");

        assert!(wait_until(Duration::from_millis(80), || {
            pads.lock().expect("lock pads").iter().any(|snapshot| {
                snapshot.route_target
                    == OhMyGamepadRouteTargetDto::StreamSession {
                        session_id: "session-1".to_owned(),
                    }
            })
        }));

        runtime.shutdown().expect("runtime should shutdown cleanly");
    }

    #[test]
    fn runtime_thread_updates_sampling_snapshot() {
        let backend = ScriptedBackend::new(vec![BackendPollResult {
            device_events: vec![DeviceLifecycleEvent::Added(device("pad-a"))],
            samples: vec![sample("pad-a", 10, vec![1.0])],
        }]);
        let runtime = spawn_input_runtime(
            InputCoreConfig::default(),
            backend,
            ThreadSafeUiSink::default(),
            ThreadSafeStreamSink,
        );

        runtime
            .update_sampling(ohmygamepad_protocol::OhMyGamepadSamplingConfigDto {
                backend_poll_rate_hz: 500,
                logical_pad_sample_rate_hz: 120,
                ui_push_rate_hz: 60,
                stream_push_mode: ohmygamepad_protocol::OhMyGamepadStreamPushModeDto::OnChange,
                stream_push_rate_hz: None,
            })
            .expect("sampling update should succeed");

        let snapshot = runtime
            .get_runtime_snapshot()
            .expect("runtime snapshot should be available");
        assert_eq!(snapshot.sampling.backend_poll_rate_hz, 500);
        assert_eq!(snapshot.sampling.logical_pad_sample_rate_hz, 120);

        runtime.shutdown().expect("runtime should shutdown cleanly");
    }

    #[test]
    fn runtime_thread_rebinds_logical_pad() {
        let backend = ScriptedBackend::new(vec![BackendPollResult {
            device_events: vec![
                DeviceLifecycleEvent::Added(device("pad-a")),
                DeviceLifecycleEvent::Added(device("pad-b")),
            ],
            samples: vec![
                sample("pad-a", 10, vec![1.0]),
                sample("pad-b", 20, vec![0.0, 1.0]),
            ],
        }]);
        let runtime = spawn_input_runtime(
            InputCoreConfig::default(),
            backend,
            ThreadSafeUiSink::default(),
            ThreadSafeStreamSink,
        );

        runtime
            .rebind_logical_pad(ohmygamepad_protocol::LogicalPadBindingDto {
                pad_id: ohmygamepad_protocol::LogicalPadId::Pad0,
                mode: ohmygamepad_protocol::OhMyGamepadBindingModeDto::FixedDevice,
                device_ids: vec!["pad-b".to_owned()],
            })
            .expect("rebind should succeed");

        let snapshot = runtime
            .get_runtime_snapshot()
            .expect("runtime snapshot should be available");
        assert_eq!(snapshot.bindings.len(), 1);
        assert_eq!(snapshot.bindings[0].device_ids, vec!["pad-b".to_owned()]);

        runtime.shutdown().expect("runtime should shutdown cleanly");
    }

    #[test]
    fn runtime_thread_replaces_device_profiles() {
        let backend = ScriptedBackend::new(vec![BackendPollResult {
            device_events: vec![DeviceLifecycleEvent::Added(device("pad-a"))],
            samples: vec![sample("pad-a", 10, vec![0.0, 0.0, 0.0, 1.0])],
        }]);
        let runtime = spawn_input_runtime(
            InputCoreConfig::default(),
            backend,
            ThreadSafeUiSink::default(),
            ThreadSafeStreamSink,
        );

        runtime
            .replace_device_profiles(vec![crate::DeviceProfile {
                matcher: DeviceProfileMatcher {
                    device_id: Some("pad-a".to_owned()),
                    ..DeviceProfileMatcher::default()
                },
                buttons: ButtonMapping {
                    south: 3,
                    north: 0,
                    ..ButtonMapping::default()
                },
                filter: FilterConfig::default(),
                ..crate::DeviceProfile::default()
            }])
            .expect("device profiles update should succeed");

        let snapshot = runtime
            .get_runtime_snapshot()
            .expect("runtime snapshot should be available");
        assert_eq!(snapshot.pads.len(), 1);
        assert_eq!(snapshot.pads[0].state.buttons.south, 1.0);
        assert_eq!(snapshot.pads[0].state.buttons.north, 0.0);

        runtime.shutdown().expect("runtime should shutdown cleanly");
    }

    #[test]
    fn runtime_snapshot_subscription_receives_initial_and_updated_snapshots() {
        let backend = ScriptedBackend::new(vec![BackendPollResult {
            device_events: vec![DeviceLifecycleEvent::Added(device("pad-a"))],
            samples: vec![sample("pad-a", 10, vec![1.0])],
        }]);
        let runtime = spawn_input_runtime(
            InputCoreConfig::default(),
            backend,
            ThreadSafeUiSink::default(),
            ThreadSafeStreamSink,
        );
        let snapshot_rx = runtime.subscribe_runtime_snapshot();

        let initial_snapshot = snapshot_rx
            .recv_timeout(Duration::from_millis(100))
            .expect("initial runtime snapshot should be pushed");
        assert_eq!(initial_snapshot.devices.len(), 0);

        let discovered_snapshot = snapshot_rx
            .recv_timeout(Duration::from_millis(100))
            .expect("device snapshot should be pushed");
        assert_eq!(discovered_snapshot.devices.len(), 1);
        assert_eq!(discovered_snapshot.devices[0].device_id, "pad-a");

        runtime
            .set_route_target(OhMyGamepadRouteTargetDto::StreamSession {
                session_id: "session-1".to_owned(),
            })
            .expect("route update should succeed");
        let routed_snapshot = snapshot_rx
            .recv_timeout(Duration::from_millis(100))
            .expect("route target snapshot should be pushed");
        assert_eq!(
            routed_snapshot.route_target,
            OhMyGamepadRouteTargetDto::StreamSession {
                session_id: "session-1".to_owned(),
            }
        );

        runtime.shutdown().expect("runtime should shutdown cleanly");
    }
}
