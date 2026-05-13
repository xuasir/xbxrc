use std::{
    sync::{
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use ohmygamepad_protocol::{
    LogicalPadBindingDto, OhMyGamepadRuntimeSnapshotDto, OhMyGamepadSamplingConfigDto,
    OhMyGamepadSamplingHealthDto, OhMyGamepadSamplingLifecycleDto,
};

use crate::{DeviceProfile, InputBackend, InputCore, InputCoreConfig, StreamSink, UiSink};

/// 逻辑采样长期不推进，但后端仍在摄入原始样本时判定为 stalled（毫秒）。
const SAMPLING_STALL_AFTER_MS: u64 = 2500;
/// 后端样本时间戳与当前时钟差小于该值视为“后端仍活跃”（毫秒）。
const SAMPLING_BACKEND_FRESH_WITHIN_MS: u64 = 900;
/// 已连接设备但从未产生 logical progress 时，超过该时钟阈值才判 stalled（毫秒）。
const SAMPLING_FIRST_PROGRESS_GRACE_MS: u64 = 3500;

fn snapshot_has_established_slot_baseline(snapshot: &OhMyGamepadRuntimeSnapshotDto) -> bool {
    snapshot
        .slots
        .iter()
        .any(|slot| slot.sampled_at_ms > 0 || slot.sample_seq > 0)
}

fn effective_pad_sample_lifecycle(
    sampling_suspended: bool,
    sampling_lifecycle: OhMyGamepadSamplingLifecycleDto,
) -> OhMyGamepadSamplingLifecycleDto {
    if sampling_suspended {
        OhMyGamepadSamplingLifecycleDto::BackgroundWarm
    } else {
        sampling_lifecycle
    }
}

fn evaluate_sampling_health(
    sampling_suspended: bool,
    clock_ms: u64,
    snapshot: &OhMyGamepadRuntimeSnapshotDto,
) -> OhMyGamepadSamplingHealthDto {
    if sampling_suspended {
        return OhMyGamepadSamplingHealthDto::Healthy;
    }
    let connected = snapshot.devices.iter().any(|device| device.connected);
    if !connected {
        return OhMyGamepadSamplingHealthDto::Healthy;
    }

    let lp = snapshot.last_sample_progress_at_ms;
    let lb = snapshot.last_backend_sample_activity_at_ms;
    let backend_fresh = lb > 0 && clock_ms.saturating_sub(lb) < SAMPLING_BACKEND_FRESH_WITHIN_MS;
    if lp == 0 {
        if lb > 0 && backend_fresh && clock_ms >= SAMPLING_FIRST_PROGRESS_GRACE_MS {
            return OhMyGamepadSamplingHealthDto::Stalled;
        }
        return OhMyGamepadSamplingHealthDto::AwaitingBaseline;
    }

    let has_slot_baseline = snapshot_has_established_slot_baseline(snapshot);

    // logical progress 已经建立后，持续刷新的 backend + 已建立的 slot baseline
    // 表示当前只是停在中性态，没有必要误判成 awaitingBaseline / stalled。
    if backend_fresh && has_slot_baseline {
        return OhMyGamepadSamplingHealthDto::Healthy;
    }

    if clock_ms.saturating_sub(lp) > SAMPLING_STALL_AFTER_MS && backend_fresh {
        return OhMyGamepadSamplingHealthDto::Stalled;
    }

    OhMyGamepadSamplingHealthDto::Healthy
}

/// 避免仅因 `clock_ms` 推进导致 `sampling_health` 抖动，从而破坏 UI 推送节流与 broadcaster 去重。
#[derive(Clone, Debug)]
struct SamplingHealthEvalCache {
    last_eval_clock_ms: u64,
    last_progress_mark: (u64, u64),
    health: OhMyGamepadSamplingHealthDto,
}

impl Default for SamplingHealthEvalCache {
    fn default() -> Self {
        Self {
            last_eval_clock_ms: 0,
            last_progress_mark: (0, 0),
            health: OhMyGamepadSamplingHealthDto::Healthy,
        }
    }
}

impl SamplingHealthEvalCache {
    const REEVAL_INTERVAL_MS: u64 = 300;

    fn refresh<TBackend, TUiSink, TStreamSink>(
        &mut self,
        core: &InputCore<TBackend, TUiSink, TStreamSink>,
        sampling_suspended: bool,
        _lifecycle: OhMyGamepadSamplingLifecycleDto,
    ) -> OhMyGamepadSamplingHealthDto
    where
        TBackend: InputBackend,
        TUiSink: UiSink,
        TStreamSink: StreamSink,
    {
        let base = core.runtime_snapshot();
        let clock_ms = core.clock_ms();
        let max_seq = base
            .slots
            .iter()
            .map(|pad| pad.sample_seq)
            .max()
            .unwrap_or(0);
        let max_sampled_at = base
            .slots
            .iter()
            .map(|pad| pad.sampled_at_ms)
            .max()
            .unwrap_or(0);
        let mark = (max_seq, max_sampled_at);
        if mark != self.last_progress_mark
            || clock_ms.saturating_sub(self.last_eval_clock_ms) >= Self::REEVAL_INTERVAL_MS
        {
            self.health = evaluate_sampling_health(sampling_suspended, clock_ms, &base);
            self.last_progress_mark = mark;
            self.last_eval_clock_ms = clock_ms;
        }
        self.health
    }
}

fn decorate_runtime_snapshot<TBackend, TUiSink, TStreamSink>(
    core: &InputCore<TBackend, TUiSink, TStreamSink>,
    sampling_lifecycle: OhMyGamepadSamplingLifecycleDto,
    sampling_suspended: bool,
    self_heal_count: u32,
    health_cache: &mut SamplingHealthEvalCache,
) -> OhMyGamepadRuntimeSnapshotDto
where
    TBackend: InputBackend,
    TUiSink: UiSink,
    TStreamSink: StreamSink,
{
    let mut snapshot = core.runtime_snapshot();
    snapshot.sampling_self_heal_count = self_heal_count;
    snapshot.sampling_lifecycle =
        effective_pad_sample_lifecycle(sampling_suspended, sampling_lifecycle);
    snapshot.sampling_health = health_cache.refresh(core, sampling_suspended, sampling_lifecycle);
    snapshot
}

fn snapshot_for_query<TBackend, TUiSink, TStreamSink>(
    core: &InputCore<TBackend, TUiSink, TStreamSink>,
    sampling_lifecycle: OhMyGamepadSamplingLifecycleDto,
    sampling_suspended: bool,
    self_heal_count: u32,
) -> OhMyGamepadRuntimeSnapshotDto
where
    TBackend: InputBackend,
    TUiSink: UiSink,
    TStreamSink: StreamSink,
{
    let mut snapshot = core.runtime_snapshot();
    snapshot.sampling_self_heal_count = self_heal_count;
    snapshot.sampling_lifecycle =
        effective_pad_sample_lifecycle(sampling_suspended, sampling_lifecycle);
    snapshot.sampling_health =
        evaluate_sampling_health(sampling_suspended, core.clock_ms(), &snapshot);
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
    SetSuspended {
        suspended: bool,
    },
    SetSamplingLifecycle {
        lifecycle: OhMyGamepadSamplingLifecycleDto,
    },
    BumpSamplingSelfHealCount,
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

    pub fn set_suspended(&self, suspended: bool) -> Result<(), InputRuntimeError> {
        self.command_tx
            .send(RuntimeCommand::SetSuspended { suspended })
            .map_err(|_| InputRuntimeError::CommandChannelClosed)
    }

    pub fn set_sampling_lifecycle(
        &self,
        lifecycle: OhMyGamepadSamplingLifecycleDto,
    ) -> Result<(), InputRuntimeError> {
        self.command_tx
            .send(RuntimeCommand::SetSamplingLifecycle { lifecycle })
            .map_err(|_| InputRuntimeError::CommandChannelClosed)
    }

    pub fn bump_sampling_self_heal_count(&self) -> Result<(), InputRuntimeError> {
        self.command_tx
            .send(RuntimeCommand::BumpSamplingSelfHealCount)
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
        let mut health_cache = SamplingHealthEvalCache::default();
        runtime_snapshot_broadcaster.publish(decorate_runtime_snapshot(
            &core,
            OhMyGamepadSamplingLifecycleDto::Active,
            false,
            0,
            &mut health_cache,
        ));
        run_runtime_loop(
            &mut core,
            &command_rx,
            origin,
            &mut schedule,
            &runtime_snapshot_broadcaster,
            &mut health_cache,
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
    health_cache: &mut SamplingHealthEvalCache,
) where
    TBackend: InputBackend,
    TUiSink: UiSink,
    TStreamSink: StreamSink,
{
    let mut sampling_lifecycle = OhMyGamepadSamplingLifecycleDto::Active;
    let mut sampling_suspended = false;
    let mut sampling_self_heal_count: u32 = 0;
    let mut publish_state = RuntimePublishState::default();
    loop {
        let now = origin.elapsed();
        flush_pending_ui_publish(snapshot_broadcaster, &mut publish_state, now);

        if sampling_suspended {
            // set_suspended(true)：不执行逻辑采样，仅轮询后端排空事件队列。
            core.sync_clock_ms(now.as_millis() as u64);
            core.poll_backend();
        } else if apply_due_actions(
            core,
            schedule.take_due(now),
            snapshot_broadcaster,
            &mut publish_state,
            now,
            sampling_lifecycle,
            sampling_suspended,
            sampling_self_heal_count,
            health_cache,
        ) {
            continue;
        }

        let timeout = if sampling_suspended {
            Duration::from_millis(100)
        } else {
            next_runtime_deadline(schedule.next_deadline(), &publish_state)
                .checked_sub(origin.elapsed())
                .unwrap_or_default()
        };

        match command_rx.recv_timeout(timeout) {
            Ok(command) => {
                let prev_lifecycle = sampling_lifecycle;
                let prev_suspended = sampling_suspended;
                if handle_runtime_command(
                    core,
                    schedule,
                    origin.elapsed(),
                    command,
                    snapshot_broadcaster,
                    &mut publish_state,
                    &mut sampling_lifecycle,
                    &mut sampling_suspended,
                    &mut sampling_self_heal_count,
                    health_cache,
                ) {
                    break;
                }

                apply_sampling_suspended_transition(
                    core,
                    schedule,
                    origin.elapsed(),
                    snapshot_broadcaster,
                    &mut publish_state,
                    prev_suspended,
                    sampling_suspended,
                    sampling_lifecycle,
                    sampling_self_heal_count,
                    health_cache,
                );
                apply_sampling_lifecycle_transition(core, prev_lifecycle, sampling_lifecycle);

                while let Ok(command) = command_rx.try_recv() {
                    let prev_inner = sampling_lifecycle;
                    let prev_susp_inner = sampling_suspended;
                    if handle_runtime_command(
                        core,
                        schedule,
                        origin.elapsed(),
                        command,
                        snapshot_broadcaster,
                        &mut publish_state,
                        &mut sampling_lifecycle,
                        &mut sampling_suspended,
                        &mut sampling_self_heal_count,
                        health_cache,
                    ) {
                        return;
                    }

                    apply_sampling_suspended_transition(
                        core,
                        schedule,
                        origin.elapsed(),
                        snapshot_broadcaster,
                        &mut publish_state,
                        prev_susp_inner,
                        sampling_suspended,
                        sampling_lifecycle,
                        sampling_self_heal_count,
                        health_cache,
                    );
                    apply_sampling_lifecycle_transition(core, prev_inner, sampling_lifecycle);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn apply_sampling_suspended_transition<TBackend, TUiSink, TStreamSink>(
    core: &mut InputCore<TBackend, TUiSink, TStreamSink>,
    schedule: &mut SamplingSchedule,
    now: Duration,
    snapshot_broadcaster: &RuntimeSnapshotBroadcaster,
    publish_state: &mut RuntimePublishState,
    prev_suspended: bool,
    next_suspended: bool,
    sampling_lifecycle: OhMyGamepadSamplingLifecycleDto,
    sampling_self_heal_count: u32,
    health_cache: &mut SamplingHealthEvalCache,
) where
    TBackend: InputBackend,
    TUiSink: UiSink,
    TStreamSink: StreamSink,
{
    if !prev_suspended && next_suspended {
        core.reset_state();
        force_publish_runtime_snapshot(
            snapshot_broadcaster,
            publish_state,
            decorate_runtime_snapshot(
                core,
                sampling_lifecycle,
                next_suspended,
                sampling_self_heal_count,
                health_cache,
            ),
            now,
        );
    } else if prev_suspended && !next_suspended {
        schedule.update_sampling(now, &core.config().sampling);
    }
}

fn apply_sampling_lifecycle_transition<TBackend, TUiSink, TStreamSink>(
    core: &mut InputCore<TBackend, TUiSink, TStreamSink>,
    prev: OhMyGamepadSamplingLifecycleDto,
    next: OhMyGamepadSamplingLifecycleDto,
) where
    TBackend: InputBackend,
    TUiSink: UiSink,
    TStreamSink: StreamSink,
{
    if prev == OhMyGamepadSamplingLifecycleDto::BackgroundWarm
        && next == OhMyGamepadSamplingLifecycleDto::Active
    {
        core.absorb_current_state_as_baseline();
    }
}

fn apply_due_actions<TBackend, TUiSink, TStreamSink>(
    core: &mut InputCore<TBackend, TUiSink, TStreamSink>,
    actions: SamplingActions,
    snapshot_broadcaster: &RuntimeSnapshotBroadcaster,
    publish_state: &mut RuntimePublishState,
    now: Duration,
    sampling_lifecycle: OhMyGamepadSamplingLifecycleDto,
    sampling_suspended: bool,
    sampling_self_heal_count: u32,
    health_cache: &mut SamplingHealthEvalCache,
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
        let pad_lifecycle = effective_pad_sample_lifecycle(sampling_suspended, sampling_lifecycle);
        core.sample_once_for_lifecycle(pad_lifecycle);
        applied = true;
    }
    if applied {
        publish_runtime_snapshot(
            snapshot_broadcaster,
            publish_state,
            decorate_runtime_snapshot(
                core,
                sampling_lifecycle,
                sampling_suspended,
                sampling_self_heal_count,
                health_cache,
            ),
            now,
        );
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
    sampling_lifecycle: &mut OhMyGamepadSamplingLifecycleDto,
    sampling_suspended: &mut bool,
    sampling_self_heal_count: &mut u32,
    health_cache: &mut SamplingHealthEvalCache,
) -> bool
where
    TBackend: InputBackend,
    TUiSink: UiSink,
    TStreamSink: StreamSink,
{
    match command {
        RuntimeCommand::GetRuntimeSnapshot { reply_tx } => {
            let _ = reply_tx.send(snapshot_for_query(
                core,
                *sampling_lifecycle,
                *sampling_suspended,
                *sampling_self_heal_count,
            ));
            false
        }
        RuntimeCommand::RefreshSnapshot => {
            core.sync_clock_ms(now.as_millis() as u64);
            core.poll_backend();
            let pad_lifecycle =
                effective_pad_sample_lifecycle(*sampling_suspended, *sampling_lifecycle);
            core.sample_once_for_lifecycle(pad_lifecycle);
            force_publish_runtime_snapshot(
                snapshot_broadcaster,
                publish_state,
                decorate_runtime_snapshot(
                    core,
                    *sampling_lifecycle,
                    *sampling_suspended,
                    *sampling_self_heal_count,
                    health_cache,
                ),
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
                decorate_runtime_snapshot(
                    core,
                    *sampling_lifecycle,
                    *sampling_suspended,
                    *sampling_self_heal_count,
                    health_cache,
                ),
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
            let pad_lifecycle =
                effective_pad_sample_lifecycle(*sampling_suspended, *sampling_lifecycle);
            core.sample_once_for_lifecycle(pad_lifecycle);
            force_publish_runtime_snapshot(
                snapshot_broadcaster,
                publish_state,
                decorate_runtime_snapshot(
                    core,
                    *sampling_lifecycle,
                    *sampling_suspended,
                    *sampling_self_heal_count,
                    health_cache,
                ),
                now,
            );
            false
        }
        RuntimeCommand::ReplaceDeviceProfiles { profiles } => {
            core.sync_clock_ms(now.as_millis() as u64);
            core.replace_device_profiles(profiles);
            let pad_lifecycle =
                effective_pad_sample_lifecycle(*sampling_suspended, *sampling_lifecycle);
            core.sample_once_for_lifecycle(pad_lifecycle);
            force_publish_runtime_snapshot(
                snapshot_broadcaster,
                publish_state,
                decorate_runtime_snapshot(
                    core,
                    *sampling_lifecycle,
                    *sampling_suspended,
                    *sampling_self_heal_count,
                    health_cache,
                ),
                now,
            );
            false
        }
        RuntimeCommand::SetSuspended {
            suspended: next_suspended,
        } => {
            *sampling_suspended = next_suspended;
            if !next_suspended {
                *sampling_lifecycle = OhMyGamepadSamplingLifecycleDto::Active;
            }
            false
        }
        RuntimeCommand::SetSamplingLifecycle { lifecycle } => {
            *sampling_lifecycle = lifecycle;
            false
        }
        RuntimeCommand::BumpSamplingSelfHealCount => {
            *sampling_self_heal_count = sampling_self_heal_count.saturating_add(1);
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
        && a.slot_bindings == b.slot_bindings
        && a.sampling == b.sampling
        && a.slots == b.slots
        && a.haptics == b.haptics
        && a.sampling_lifecycle == b.sampling_lifecycle
        && a.sampling_health == b.sampling_health
        && a.sampling_self_heal_count == b.sampling_self_heal_count
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
