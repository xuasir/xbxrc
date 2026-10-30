use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use arc_swap::ArcSwap;
use ohmygamepad_protocol::{
    LogicalPadBindingDto, LogicalPadSnapshotDto, OhMyGamepadRuntimeSnapshotDto,
    OhMyGamepadSamplingConfigDto, OhMyGamepadSamplingHealthDto, OhMyGamepadSamplingLifecycleDto,
};

use crate::{
    DeviceProfile, InputBackend, InputCore, InputCoreConfig, InputProcessingMode, StreamSink,
    UiSink,
};

use super::monotonic_origin;

fn decorate_runtime_snapshot<TBackend, TUiSink, TStreamSink>(
    core: &InputCore<TBackend, TUiSink, TStreamSink>,
) -> OhMyGamepadRuntimeSnapshotDto
where
    TBackend: InputBackend,
    TUiSink: UiSink,
    TStreamSink: StreamSink,
{
    let mut snapshot = core.runtime_snapshot();
    // 采样常驻运行：诊断字段固定为健康值，不再依赖壳层生命周期驱动。
    snapshot.sampling_lifecycle = OhMyGamepadSamplingLifecycleDto::Active;
    snapshot.sampling_health = OhMyGamepadSamplingHealthDto::Healthy;
    snapshot.sampling_self_heal_count = 0;
    snapshot
}

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
    RefreshSnapshot,
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
    SourceCommandFailed,
    HapticsUnavailable,
    HapticsTransportFailed,
}

pub struct InputRuntimeHandle {
    command_tx: Sender<RuntimeCommand>,
    snapshot_broadcaster: Arc<RuntimeSnapshotBroadcaster>,
    /// 无锁最新 pad 状态单元：每次 sample 后整体 replace；高频读方（如串流输入环）
    /// 直接 load 读取，避免对 runtime 线程的阻塞往返。
    latest_pad_states: Arc<ArcSwap<Vec<LogicalPadSnapshotDto>>>,
    /// 单调递增变更计数：每次 replace 后 bump。生产读方直接 `latest_pad_states().load()`
    /// 即可拿到最新值，无需消费 seq；seq 仅供测试/诊断确认「采样在持续推进」。
    latest_pad_seq: Arc<AtomicU64>,
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

    pub fn refresh_snapshot(&self) -> Result<(), InputRuntimeError> {
        self.command_tx
            .send(RuntimeCommand::RefreshSnapshot)
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

    /// 暴露给高频读方（串流输入环）的最新 pad 状态单元；load 为 lock-free 原子读。
    pub fn latest_pad_states(&self) -> Arc<ArcSwap<Vec<LogicalPadSnapshotDto>>> {
        Arc::clone(&self.latest_pad_states)
    }

    /// 单调递增变更计数（测试/诊断用）：每次 ArcSwap replace 后 bump 一次；
    /// 生产读方直接 `latest_pad_states().load()`，无需消费此 seq。
    pub fn latest_pad_seq(&self) -> u64 {
        self.latest_pad_seq.load(Ordering::Acquire)
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
    let latest_pad_states = Arc::new(ArcSwap::from_pointee(Vec::<LogicalPadSnapshotDto>::new()));
    let runtime_latest_pad_states = Arc::clone(&latest_pad_states);
    let latest_pad_seq = Arc::new(AtomicU64::new(0));
    let runtime_latest_pad_seq = Arc::clone(&latest_pad_seq);
    let join_handle = thread::spawn(move || {
        let mut core = InputCore::new(config, backend, ui_sink, stream_sink);
        let origin = monotonic_origin();
        let mut schedule = SamplingSchedule::new(&core.config().sampling);
        let initial_snapshot = decorate_runtime_snapshot(&core);
        runtime_snapshot_broadcaster.publish(initial_snapshot.clone());
        replace_latest_pad_states(
            &runtime_latest_pad_states,
            &runtime_latest_pad_seq,
            &initial_snapshot,
            core.config().processing_mode,
        );
        run_runtime_loop(
            &mut core,
            &command_rx,
            origin,
            &mut schedule,
            &runtime_snapshot_broadcaster,
            &runtime_latest_pad_states,
            &runtime_latest_pad_seq,
        );
    });

    InputRuntimeHandle {
        command_tx,
        snapshot_broadcaster,
        latest_pad_states,
        latest_pad_seq,
        join_handle: Some(join_handle),
    }
}

/// 从完整快照中提取各 slot 的最新 LogicalPadSnapshot（= GamepadSlotSnapshotDto），
/// 整体 replace 到 ArcSwap，并 bump 单调变更计数（seq）。高频读方（串流输入环）
/// 直接 load 最新状态，seq 只作测试/诊断的推进证据。
fn replace_latest_pad_states(
    cell: &Arc<ArcSwap<Vec<LogicalPadSnapshotDto>>>,
    seq: &Arc<AtomicU64>,
    snapshot: &OhMyGamepadRuntimeSnapshotDto,
    mode: InputProcessingMode,
) {
    if mode == InputProcessingMode::StandardDevices {
        return;
    }
    let states = snapshot.slots.clone();
    cell.store(Arc::new(states));
    seq.fetch_add(1, Ordering::Release);
}

fn run_runtime_loop<TBackend, TUiSink, TStreamSink>(
    core: &mut InputCore<TBackend, TUiSink, TStreamSink>,
    command_rx: &Receiver<RuntimeCommand>,
    origin: Instant,
    schedule: &mut SamplingSchedule,
    snapshot_broadcaster: &RuntimeSnapshotBroadcaster,
    latest_pad_states: &Arc<ArcSwap<Vec<LogicalPadSnapshotDto>>>,
    latest_pad_seq: &Arc<AtomicU64>,
) where
    TBackend: InputBackend,
    TUiSink: UiSink,
    TStreamSink: StreamSink,
{
    let mut publish_state = RuntimePublishState::default();
    loop {
        let now = origin.elapsed();
        flush_pending_ui_publish(snapshot_broadcaster, &mut publish_state, now);

        if apply_due_actions(
            core,
            schedule.take_due(now),
            snapshot_broadcaster,
            latest_pad_states,
            latest_pad_seq,
            &mut publish_state,
            now,
        ) {
            continue;
        }

        let timeout = next_runtime_deadline(schedule.next_deadline(), &publish_state)
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
                    latest_pad_states,
                    latest_pad_seq,
                    &mut publish_state,
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
                        latest_pad_states,
                        latest_pad_seq,
                        &mut publish_state,
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
    latest_pad_states: &Arc<ArcSwap<Vec<LogicalPadSnapshotDto>>>,
    latest_pad_seq: &Arc<AtomicU64>,
    publish_state: &mut RuntimePublishState,
    now: Duration,
) -> bool
where
    TBackend: InputBackend,
    TUiSink: UiSink,
    TStreamSink: StreamSink,
{
    let mut applied = false;
    let mut did_sample = false;
    if actions.poll_backend {
        core.sync_clock_ms(now.as_millis() as u64);
        core.poll_backend();
        applied = true;
    }
    if actions.sample_pads && core.config().processing_mode == InputProcessingMode::LogicalPads {
        core.sync_clock_ms(now.as_millis() as u64);
        core.sample_once();
        applied = true;
        did_sample = true;
    }
    if applied {
        let snapshot = decorate_runtime_snapshot(core);
        if did_sample {
            // 高频读方独立于 UI 节流：每次真正 sample 都 replace 一次 ArcSwap，
            // 避免 input loop 读到的 pad 状态比 snapshot_broadcaster 推送还旧。
            replace_latest_pad_states(
                latest_pad_states,
                latest_pad_seq,
                &snapshot,
                core.config().processing_mode,
            );
        }
        publish_runtime_snapshot(snapshot_broadcaster, publish_state, snapshot, now);
    }
    applied
}

fn handle_runtime_command<TBackend, TUiSink, TStreamSink>(
    core: &mut InputCore<TBackend, TUiSink, TStreamSink>,
    schedule: &mut SamplingSchedule,
    now: Duration,
    command: RuntimeCommand,
    snapshot_broadcaster: &RuntimeSnapshotBroadcaster,
    latest_pad_states: &Arc<ArcSwap<Vec<LogicalPadSnapshotDto>>>,
    latest_pad_seq: &Arc<AtomicU64>,
    publish_state: &mut RuntimePublishState,
) -> bool
where
    TBackend: InputBackend,
    TUiSink: UiSink,
    TStreamSink: StreamSink,
{
    match command {
        RuntimeCommand::GetRuntimeSnapshot { reply_tx } => {
            let _ = reply_tx.send(decorate_runtime_snapshot(core));
            false
        }
        RuntimeCommand::RefreshSnapshot => {
            core.sync_clock_ms(now.as_millis() as u64);
            core.poll_backend();
            core.sample_once();
            let snapshot = decorate_runtime_snapshot(core);
            replace_latest_pad_states(
                latest_pad_states,
                latest_pad_seq,
                &snapshot,
                core.config().processing_mode,
            );
            force_publish_runtime_snapshot(snapshot_broadcaster, publish_state, snapshot, now);
            false
        }
        RuntimeCommand::UpdateSampling { sampling } => {
            core.replace_sampling_config(sampling);
            schedule.update_sampling(now, &core.config().sampling);
            force_publish_runtime_snapshot(
                snapshot_broadcaster,
                publish_state,
                decorate_runtime_snapshot(core),
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
            let snapshot = decorate_runtime_snapshot(core);
            replace_latest_pad_states(
                latest_pad_states,
                latest_pad_seq,
                &snapshot,
                core.config().processing_mode,
            );
            force_publish_runtime_snapshot(snapshot_broadcaster, publish_state, snapshot, now);
            false
        }
        RuntimeCommand::ReplaceDeviceProfiles { profiles } => {
            core.sync_clock_ms(now.as_millis() as u64);
            core.replace_device_profiles(profiles);
            core.sample_once();
            let snapshot = decorate_runtime_snapshot(core);
            replace_latest_pad_states(
                latest_pad_states,
                latest_pad_seq,
                &snapshot,
                core.config().processing_mode,
            );
            force_publish_runtime_snapshot(snapshot_broadcaster, publish_state, snapshot, now);
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

fn runtime_snapshot_broadcast_semantic_eq(
    a: &OhMyGamepadRuntimeSnapshotDto,
    b: &OhMyGamepadRuntimeSnapshotDto,
) -> bool {
    a.devices == b.devices
        && a.device_samples == b.device_samples
        && a.slot_bindings == b.slot_bindings
        && a.sampling == b.sampling
        && a.slots == b.slots
        && a.haptics == b.haptics
        && a.sampling_lifecycle == b.sampling_lifecycle
        && a.sampling_health == b.sampling_health
        && a.sampling_self_heal_count == b.sampling_self_heal_count
        && a.stream_pad_forwarding == b.stream_pad_forwarding
        && a.input_route == b.input_route
}

fn merge_sampling_diagnostic_timestamps(
    target: &mut OhMyGamepadRuntimeSnapshotDto,
    source: &OhMyGamepadRuntimeSnapshotDto,
) {
    target.last_sample_progress_at_ms = source.last_sample_progress_at_ms;
    target.last_backend_sample_activity_at_ms = source.last_backend_sample_activity_at_ms;
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
        if let Some(current) = state.current_snapshot.as_mut() {
            if runtime_snapshot_broadcast_semantic_eq(current, &snapshot) {
                merge_sampling_diagnostic_timestamps(current, &snapshot);
                return;
            }
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
