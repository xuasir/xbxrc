use std::{
    sync::{
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use ohmygamepad_protocol::{
    LogicalPadBindingDto, OhMyGamepadInputPolicyDto, OhMyGamepadRuntimeSnapshotDto,
    OhMyGamepadSamplingConfigDto,
};

use crate::{DeviceProfile, InputBackend, InputCore, InputCoreConfig, StreamSink, UiSink};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SamplingActions {
    poll_backend: bool,
    sample_pads: bool,
}

#[derive(Clone, Debug, Default)]
struct RuntimePublishState {
    last_ui_publish_at: Option<Duration>,
    pending_snapshot: Option<OhMyGamepadRuntimeSnapshotDto>,
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
    SetInputPolicy {
        policy: OhMyGamepadInputPolicyDto,
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
    SetSuspended {
        suspended: bool,
    },
    Shutdown,
}

#[derive(Debug, Eq, PartialEq)]
pub enum InputRuntimeError {
    CommandChannelClosed,
    ResponseChannelClosed,
    ThreadJoinFailed,
    HapticsUnavailable,
    HapticsTransportFailed,
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

    pub fn set_input_policy(
        &self,
        policy: OhMyGamepadInputPolicyDto,
    ) -> Result<(), InputRuntimeError> {
        self.command_tx
            .send(RuntimeCommand::SetInputPolicy { policy })
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

    pub fn set_suspended(&self, suspended: bool) -> Result<(), InputRuntimeError> {
        self.command_tx
            .send(RuntimeCommand::SetSuspended { suspended })
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
    let mut suspended = false;
    let mut publish_state = RuntimePublishState::default();
    loop {
        let now = origin.elapsed();
        flush_pending_ui_publish(snapshot_broadcaster, &mut publish_state, now);

        if suspended {
            // 挂起状态下：
            // 1. 不执行逻辑采样 (sample_pads)
            // 2. 依然执行后端轮询 (poll_backend) 以排空系统事件队列，防止切回时产生指令积压（Backlog）。
            // 3. 但不更新 logical pads 或发布新快照。
            core.sync_clock_ms(now.as_millis() as u64);
            core.poll_backend();
        } else if apply_due_actions(
            core,
            schedule.take_due(now),
            snapshot_broadcaster,
            &mut publish_state,
            now,
        ) {
            continue;
        }

        let timeout = if suspended {
            Duration::from_millis(100) // 挂起时大幅降低检查频率
        } else {
            next_runtime_deadline(schedule.next_deadline(), &publish_state)
                .checked_sub(origin.elapsed())
                .unwrap_or_default()
        };

        match command_rx.recv_timeout(timeout) {
            Ok(command) => {
                let was_suspended = suspended;
                if handle_runtime_command(
                    core,
                    schedule,
                    origin.elapsed(),
                    command,
                    snapshot_broadcaster,
                    &mut publish_state,
                    &mut suspended,
                ) {
                    break;
                }

                // 状态转换逻辑：切入挂起时立即重置
                if !was_suspended && suspended {
                    core.reset_state();
                    snapshot_broadcaster.publish(core.runtime_snapshot());
                }
                // 切回活跃时：重置调度器，从当前时间点重新起跳
                else if was_suspended && !suspended {
                    schedule.update_sampling(origin.elapsed(), &core.config().sampling);
                }

                while let Ok(command) = command_rx.try_recv() {
                    let was_suspended_inner = suspended;
                    if handle_runtime_command(
                        core,
                        schedule,
                        origin.elapsed(),
                        command,
                        snapshot_broadcaster,
                        &mut publish_state,
                        &mut suspended,
                    ) {
                        return;
                    }

                    if !was_suspended_inner && suspended {
                        core.reset_state();
                        snapshot_broadcaster.publish(core.runtime_snapshot());
                    } else if was_suspended_inner && !suspended {
                        schedule.update_sampling(origin.elapsed(), &core.config().sampling);
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
    publish_state: &mut RuntimePublishState,
    now: Duration,
) -> bool
where
    TBackend: InputBackend,
    TUiSink: UiSink,
    TStreamSink: StreamSink,
{
    let mut applied = false;
    if actions.poll_backend {
        core.sync_clock_ms(now.as_millis() as u64);
        core.poll_backend();
        applied = true;
    }
    if actions.sample_pads {
        core.sync_clock_ms(now.as_millis() as u64);
        core.sample_once();
        applied = true;
    }
    if applied {
        publish_runtime_snapshot(snapshot_broadcaster, publish_state, core.runtime_snapshot(), now);
    }
    applied
}

fn handle_runtime_command<TBackend, TUiSink, TStreamSink>(
    core: &mut InputCore<TBackend, TUiSink, TStreamSink>,
    schedule: &mut SamplingSchedule,
    now: Duration,
    command: RuntimeCommand,
    snapshot_broadcaster: &RuntimeSnapshotBroadcaster,
    publish_state: &mut RuntimePublishState,
    suspended: &mut bool,
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
        RuntimeCommand::SetInputPolicy { policy } => {
            core.sync_clock_ms(now.as_millis() as u64);
            core.replace_input_policy(policy);
            core.sample_once();
            force_publish_runtime_snapshot(
                snapshot_broadcaster,
                publish_state,
                core.runtime_snapshot(),
                now,
            );
            false
        }
        RuntimeCommand::UpdateSampling { sampling } => {
            core.replace_sampling_config(sampling);
            schedule.update_sampling(now, &core.config().sampling);
            force_publish_runtime_snapshot(
                snapshot_broadcaster,
                publish_state,
                core.runtime_snapshot(),
                now,
            );
            false
        }
        RuntimeCommand::RebindLogicalPad { binding } => {
            let mut bindings = core.config().bindings.clone();
            if let Some(index) = bindings.iter().position(|item| item.slot == binding.slot) {
                bindings[index] = binding;
            } else {
                bindings.push(binding);
            }
            core.sync_clock_ms(now.as_millis() as u64);
            core.replace_bindings(bindings);
            core.sample_once();
            force_publish_runtime_snapshot(
                snapshot_broadcaster,
                publish_state,
                core.runtime_snapshot(),
                now,
            );
            false
        }
        RuntimeCommand::ReplaceDeviceProfiles { profiles } => {
            core.sync_clock_ms(now.as_millis() as u64);
            core.replace_device_profiles(profiles);
            core.sample_once();
            force_publish_runtime_snapshot(
                snapshot_broadcaster,
                publish_state,
                core.runtime_snapshot(),
                now,
            );
            false
        }
        RuntimeCommand::SetSuspended {
            suspended: next_suspended,
        } => {
            *suspended = next_suspended;
            false
        }
        RuntimeCommand::Shutdown => true,
    }
}

fn next_runtime_deadline(
    sampling_deadline: Duration,
    publish_state: &RuntimePublishState,
) -> Duration {
    match pending_ui_publish_deadline(publish_state) {
        Some(deadline) => sampling_deadline.min(deadline),
        None => sampling_deadline,
    }
}

fn publish_runtime_snapshot(
    snapshot_broadcaster: &RuntimeSnapshotBroadcaster,
    publish_state: &mut RuntimePublishState,
    snapshot: OhMyGamepadRuntimeSnapshotDto,
    now: Duration,
) {
    if should_publish_ui_snapshot(publish_state, &snapshot, now) {
        snapshot_broadcaster.publish(snapshot);
        publish_state.last_ui_publish_at = Some(now);
        publish_state.pending_snapshot = None;
        return;
    }

    publish_state.pending_snapshot = Some(snapshot);
}

fn force_publish_runtime_snapshot(
    snapshot_broadcaster: &RuntimeSnapshotBroadcaster,
    publish_state: &mut RuntimePublishState,
    snapshot: OhMyGamepadRuntimeSnapshotDto,
    now: Duration,
) {
    snapshot_broadcaster.publish(snapshot);
    publish_state.last_ui_publish_at = Some(now);
    publish_state.pending_snapshot = None;
}

fn flush_pending_ui_publish(
    snapshot_broadcaster: &RuntimeSnapshotBroadcaster,
    publish_state: &mut RuntimePublishState,
    now: Duration,
) {
    let Some(deadline) = pending_ui_publish_deadline(publish_state) else {
        return;
    };
    if now < deadline {
        return;
    }
    let Some(snapshot) = publish_state.pending_snapshot.take() else {
        return;
    };
    snapshot_broadcaster.publish(snapshot);
    publish_state.last_ui_publish_at = Some(now);
}

fn should_publish_ui_snapshot(
    publish_state: &RuntimePublishState,
    snapshot: &OhMyGamepadRuntimeSnapshotDto,
    now: Duration,
) -> bool {
    let interval = interval_from_hz(snapshot.sampling.ui_push_rate_hz);
    match publish_state.last_ui_publish_at {
        None => true,
        Some(last_publish_at) => now >= last_publish_at + interval,
    }
}

fn pending_ui_publish_deadline(publish_state: &RuntimePublishState) -> Option<Duration> {
    let snapshot = publish_state.pending_snapshot.as_ref()?;
    let interval = interval_from_hz(snapshot.sampling.ui_push_rate_hz);
    publish_state
        .last_ui_publish_at
        .map(|last_publish_at| last_publish_at + interval)
        .or(Some(Duration::ZERO))
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
#[path = "runner.test.rs"]
mod tests;
