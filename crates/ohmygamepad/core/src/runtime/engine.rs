use std::collections::{HashMap, HashSet};

use ohmygamepad_protocol::{
    GamepadSlotBindingDto, GamepadSlotSnapshotDto, LogicalPadBindingDto, LogicalPadId,
    LogicalPadSnapshotDto, OhMyGamepadBindingModeDto, OhMyGamepadDeviceDto,
    OhMyGamepadInputPolicyDto, OhMyGamepadRuntimeHapticsDto, OhMyGamepadRuntimeSnapshotDto,
    OhMyGamepadSamplingConfigDto, OhMyGamepadSamplingHealthDto, OhMyGamepadSamplingLifecycleDto,
};

use crate::{
    default_logical_pad_state, map_sample_with_profile, map_standard_sample, merge_states,
    BackendPollResult, DeviceLifecycleEvent, InputBackend, InputCoreConfig, RawDeviceSample,
    StreamSink, UiSink,
};

pub struct InputCore<TBackend, TUiSink, TStreamSink> {
    config: InputCoreConfig,
    backend: TBackend,
    ui_sink: TUiSink,
    stream_sink: TStreamSink,
    devices: Vec<OhMyGamepadDeviceDto>,
    pads: Vec<LogicalPadSnapshotDto>,
    raw_samples: HashMap<String, RawDeviceSample>,
    last_active_at: HashMap<String, u64>,
    active_device_by_pad: [Option<String>; 4],
    sample_seq: u64,
    clock_ms: u64,
    last_stream_emit_at_ms: [u64; 4],
    /// Updated when `sample_seq` advances (logical sampling progress).
    last_sample_progress_at_ms: u64,
    /// Updated when backend poll ingests raw device samples.
    last_backend_sample_activity_at_ms: u64,
}

impl<TBackend, TUiSink, TStreamSink> InputCore<TBackend, TUiSink, TStreamSink>
where
    TBackend: InputBackend,
    TUiSink: UiSink,
    TStreamSink: StreamSink,
{
    pub fn new(
        config: InputCoreConfig,
        backend: TBackend,
        ui_sink: TUiSink,
        stream_sink: TStreamSink,
    ) -> Self {
        Self {
            config,
            backend,
            ui_sink,
            stream_sink,
            devices: Vec::new(),
            pads: Vec::new(),
            raw_samples: HashMap::new(),
            last_active_at: HashMap::new(),
            active_device_by_pad: [None, None, None, None],
            sample_seq: 0,
            clock_ms: 0,
            last_stream_emit_at_ms: [0; 4],
            last_sample_progress_at_ms: 0,
            last_backend_sample_activity_at_ms: 0,
        }
    }

    pub fn config(&self) -> &InputCoreConfig {
        &self.config
    }

    pub fn clock_ms(&self) -> u64 {
        self.clock_ms
    }

    pub fn runtime_snapshot(&self) -> OhMyGamepadRuntimeSnapshotDto {
        OhMyGamepadRuntimeSnapshotDto {
            devices: self.devices.clone(),
            slot_bindings: self
                .config
                .bindings
                .iter()
                .cloned()
                .map(|binding| GamepadSlotBindingDto {
                    slot: binding.slot,
                    mode: binding.mode,
                    device_ids: binding.device_ids,
                })
                .collect(),
            input_policy: self.config.input_policy,
            sampling: self.config.sampling.clone(),
            slots: self.pads.clone(),
            haptics: OhMyGamepadRuntimeHapticsDto::default(),
            sampling_lifecycle: OhMyGamepadSamplingLifecycleDto::Active,
            sampling_health: OhMyGamepadSamplingHealthDto::Healthy,
            last_sample_progress_at_ms: self.last_sample_progress_at_ms,
            last_backend_sample_activity_at_ms: self.last_backend_sample_activity_at_ms,
            sampling_self_heal_count: 0,
        }
    }

    pub fn replace_sampling_config(&mut self, sampling: OhMyGamepadSamplingConfigDto) {
        self.config.sampling = sampling;
    }

    pub fn replace_input_policy(&mut self, input_policy: OhMyGamepadInputPolicyDto) {
        self.config.input_policy = input_policy;
    }

    pub fn replace_bindings(&mut self, bindings: Vec<LogicalPadBindingDto>) {
        self.config.bindings = bindings;
    }

    pub fn replace_device_profiles(&mut self, device_profiles: Vec<crate::DeviceProfile>) {
        self.config.device_profiles = device_profiles;
    }

    pub fn tick(&mut self) {
        self.poll_backend();
        self.sample_once();
    }

    pub fn tick_for_lifecycle(&mut self, lifecycle: OhMyGamepadSamplingLifecycleDto) {
        self.poll_backend();
        self.sample_once_for_lifecycle(lifecycle);
    }

    pub fn sync_clock_ms(&mut self, now_ms: u64) {
        self.clock_ms = self.clock_ms.max(now_ms);
    }

    pub fn poll_backend(&mut self) {
        let poll = self.backend.poll();
        self.apply_backend_poll(poll);
    }

    pub fn sample_once(&mut self) {
        self.sample_once_for_lifecycle(OhMyGamepadSamplingLifecycleDto::Active);
    }

    pub fn sample_once_for_lifecycle(&mut self, lifecycle: OhMyGamepadSamplingLifecycleDto) {
        self.refresh_pad_snapshots(lifecycle);
    }

    pub fn push_pad_snapshot(&mut self, snapshot: LogicalPadSnapshotDto) {
        self.push_pad_snapshot_for_lifecycle(snapshot, OhMyGamepadSamplingLifecycleDto::Active);
    }

    pub fn push_pad_snapshot_for_lifecycle(
        &mut self,
        snapshot: LogicalPadSnapshotDto,
        lifecycle: OhMyGamepadSamplingLifecycleDto,
    ) {
        // 这里先固定按逻辑 pad 覆盖快照，后续再补采样/过滤流水线。
        if let Some(existing) = self
            .pads
            .iter_mut()
            .find(|current| current.slot == snapshot.slot)
        {
            *existing = snapshot.clone();
        } else {
            self.pads.push(snapshot.clone());
        }
        if lifecycle == OhMyGamepadSamplingLifecycleDto::Active {
            self.ui_sink.emit_pad_snapshot(&snapshot);
            self.stream_sink.emit_pad_snapshot(&snapshot);
        }
    }

    pub fn pad_snapshot(&self, pad_id: LogicalPadId) -> Option<&LogicalPadSnapshotDto> {
        self.pads.iter().find(|snapshot| snapshot.slot == pad_id)
    }

    pub fn reset_state(&mut self) {
        // 清理原始采样，防止恢复时使用过期的硬件状态
        self.raw_samples.clear();
        self.last_active_at.clear();
        self.last_stream_emit_at_ms = [self.clock_ms; 4];
        self.last_backend_sample_activity_at_ms = 0;

        // 将所有逻辑手柄重置为中性状态
        for snapshot in &mut self.pads {
            snapshot.state = default_logical_pad_state();
            self.sample_seq = self.sample_seq.saturating_add(1);
            snapshot.sample_seq = self.sample_seq;
            snapshot.sampled_at_ms = self.clock_ms;
            self.last_sample_progress_at_ms = self.clock_ms;
            // 立即向接收端同步“按键全部弹起”的状态，防止 UI 粘滞。
            self.ui_sink.emit_pad_snapshot(snapshot);
            self.stream_sink.emit_pad_snapshot(snapshot);
        }
    }

    pub fn absorb_current_state_as_baseline(&mut self) {
        for snapshot in &self.pads {
            let slot_index = pad_slot(snapshot.slot);
            self.last_stream_emit_at_ms[slot_index] = self.clock_ms.max(snapshot.sampled_at_ms);
        }
    }

    fn apply_backend_poll(&mut self, poll: BackendPollResult) {
        let BackendPollResult {
            device_events,
            samples,
        } = poll;
        let mut devices_changed = false;

        for event in device_events {
            devices_changed |= self.apply_device_event(event);
        }

        for sample in samples {
            let observed = sample.observed_at_ms.max(self.clock_ms);
            self.last_backend_sample_activity_at_ms =
                self.last_backend_sample_activity_at_ms.max(observed);
            self.track_activity(&sample);
            self.raw_samples.insert(sample.device_id.clone(), sample);
        }

        if devices_changed {
            self.ui_sink.emit_devices_changed(&self.devices);
        }
    }

    fn apply_device_event(&mut self, event: DeviceLifecycleEvent) -> bool {
        match event {
            DeviceLifecycleEvent::Added(device) | DeviceLifecycleEvent::Updated(device) => {
                self.upsert_device(device)
            }
            DeviceLifecycleEvent::Removed { device_id, .. } => {
                let previous_len = self.devices.len();
                self.devices.retain(|device| device.device_id != device_id);
                self.raw_samples.remove(&device_id);
                self.last_active_at.remove(&device_id);
                self.clear_active_device_if_matches(&device_id);
                previous_len != self.devices.len()
            }
        }
    }

    fn upsert_device(&mut self, device: OhMyGamepadDeviceDto) -> bool {
        if let Some(existing) = self
            .devices
            .iter_mut()
            .find(|current| current.device_id == device.device_id)
        {
            if *existing == device {
                return false;
            }
            *existing = device;
            return true;
        }

        self.devices.push(device);
        true
    }

    fn refresh_pad_snapshots(&mut self, lifecycle: OhMyGamepadSamplingLifecycleDto) {
        let bindings = self.effective_bindings();
        let mut next_pads = Vec::with_capacity(bindings.len());
        let mut reserved_split_devices = HashSet::new();

        for binding in &bindings {
            let selected_device_ids =
                self.resolve_binding_device_ids(binding, &mut reserved_split_devices);
            let snapshot = self.build_pad_snapshot(binding.slot, selected_device_ids);
            next_pads.push(snapshot);
        }

        next_pads.sort_by_key(|snapshot| pad_order(snapshot.slot));

        for snapshot in &next_pads {
            let changed = self
                .pads
                .iter()
                .find(|current| current.slot == snapshot.slot)
                .map(|current| pad_payload_changed(current, snapshot))
                .unwrap_or(true);
            if changed && lifecycle == OhMyGamepadSamplingLifecycleDto::Active {
                self.ui_sink.emit_pad_snapshot(snapshot);
                self.stream_sink.emit_pad_snapshot(snapshot);
            }
        }

        self.pads = next_pads;
    }

    fn effective_bindings(&self) -> Vec<LogicalPadBindingDto> {
        let mut bindings = if self.config.bindings.is_empty() {
            vec![LogicalPadBindingDto {
                slot: LogicalPadId::Pad0,
                mode: OhMyGamepadBindingModeDto::SingleActive,
                device_ids: Vec::new(),
            }]
        } else {
            self.config.bindings.clone()
        };
        bindings.sort_by_key(|binding| pad_order(binding.slot));
        bindings
    }

    fn resolve_binding_device_ids(
        &mut self,
        binding: &LogicalPadBindingDto,
        reserved_split_devices: &mut HashSet<String>,
    ) -> Vec<String> {
        let connected_ids = self.connected_device_ids();
        let candidate_ids = filter_candidate_ids(&connected_ids, &binding.device_ids);

        match binding.mode {
            OhMyGamepadBindingModeDto::SingleActive => self
                .resolve_single_active_device(binding.slot, &candidate_ids)
                .into_iter()
                .collect(),
            OhMyGamepadBindingModeDto::FixedDevice => candidate_ids.into_iter().take(1).collect(),
            OhMyGamepadBindingModeDto::Merged => candidate_ids,
            OhMyGamepadBindingModeDto::Split => {
                let selected = candidate_ids
                    .into_iter()
                    .find(|device_id| !reserved_split_devices.contains(device_id));
                if let Some(device_id) = selected {
                    reserved_split_devices.insert(device_id.clone());
                    vec![device_id]
                } else {
                    Vec::new()
                }
            }
            OhMyGamepadBindingModeDto::LastActiveFailover => self
                .resolve_failover_device(binding.slot, &candidate_ids)
                .into_iter()
                .collect(),
        }
    }

    fn resolve_single_active_device(
        &mut self,
        pad_id: LogicalPadId,
        candidate_ids: &[String],
    ) -> Option<String> {
        let resolved = candidate_ids
            .iter()
            .max_by_key(|device_id| self.last_active_at.get(*device_id).copied().unwrap_or(0))
            .cloned()
            .or_else(|| candidate_ids.first().cloned());
        self.set_active_device(pad_id, resolved.clone());
        resolved
    }

    fn resolve_failover_device(
        &mut self,
        pad_id: LogicalPadId,
        candidate_ids: &[String],
    ) -> Option<String> {
        if let Some(current) = self.active_device(pad_id) {
            if candidate_ids.iter().any(|device_id| device_id == current) {
                return Some(current.to_owned());
            }
        }

        let resolved = candidate_ids
            .iter()
            .max_by_key(|device_id| self.last_active_at.get(*device_id).copied().unwrap_or(0))
            .cloned()
            .or_else(|| candidate_ids.first().cloned());
        self.set_active_device(pad_id, resolved.clone());
        resolved
    }

    fn build_pad_snapshot(
        &mut self,
        pad_id: LogicalPadId,
        device_ids: Vec<String>,
    ) -> LogicalPadSnapshotDto {
        let raw_buttons = collect_pressed_raw_buttons(&device_ids, &self.raw_samples);
        let states = device_ids
            .iter()
            .filter_map(|device_id| self.raw_samples.get(device_id))
            .map(|sample| self.map_device_sample(sample))
            .collect::<Vec<_>>();

        let sampled_at_ms = device_ids
            .iter()
            .filter_map(|device_id| self.raw_samples.get(device_id))
            .map(|sample| sample.observed_at_ms)
            .max()
            .unwrap_or(0);

        let candidate = GamepadSlotSnapshotDto {
            slot: pad_id,
            device_ids,
            sampled_at_ms,
            sample_seq: 0,
            state: if states.is_empty() {
                default_logical_pad_state()
            } else {
                merge_states(&states)
            },
            raw_buttons,
        };

        self.resolve_stream_snapshot(candidate)
    }

    fn resolve_stream_snapshot(
        &mut self,
        candidate: LogicalPadSnapshotDto,
    ) -> LogicalPadSnapshotDto {
        let slot_index = pad_slot(candidate.slot);
        let Some(existing) = self
            .pads
            .iter()
            .find(|snapshot| snapshot.slot == candidate.slot)
        else {
            let mut next = candidate;
            next.sample_seq = self.next_sample_seq();
            next.sampled_at_ms = next.sampled_at_ms.max(self.clock_ms);
            self.last_stream_emit_at_ms[slot_index] = next.sampled_at_ms;
            return next;
        };

        if pad_payload_changed(existing, &candidate) {
            let mut next = candidate;
            next.sample_seq = self.next_sample_seq();
            next.sampled_at_ms = next.sampled_at_ms.max(self.clock_ms);
            self.last_stream_emit_at_ms[slot_index] = next.sampled_at_ms;
            return next;
        }

        if self.should_emit_stream_keepalive(slot_index) {
            let mut next = candidate;
            next.sample_seq = self.next_sample_seq();
            next.sampled_at_ms = self.clock_ms.max(next.sampled_at_ms);
            self.last_stream_emit_at_ms[slot_index] = next.sampled_at_ms;
            return next;
        }

        existing.clone()
    }

    fn should_emit_stream_keepalive(&self, slot_index: usize) -> bool {
        if self.config.sampling.stream_push_mode
            != ohmygamepad_protocol::OhMyGamepadStreamPushModeDto::FixedRate
        {
            return false;
        }

        let Some(rate_hz) = self.config.sampling.stream_push_rate_hz else {
            return false;
        };

        let interval_ms = hz_to_interval_ms(rate_hz);
        self.clock_ms
            .saturating_sub(self.last_stream_emit_at_ms[slot_index])
            >= interval_ms
    }

    fn next_sample_seq(&mut self) -> u64 {
        self.sample_seq = self.sample_seq.saturating_add(1);
        self.last_sample_progress_at_ms = self.clock_ms;
        self.sample_seq
    }

    fn connected_device_ids(&self) -> Vec<String> {
        self.devices
            .iter()
            .filter(|device| device.connected)
            .map(|device| device.device_id.clone())
            .collect()
    }

    fn map_device_sample(
        &self,
        sample: &RawDeviceSample,
    ) -> ohmygamepad_protocol::LogicalPadStateDto {
        if let Some(profile) = self.find_matching_profile(&sample.device_id) {
            map_sample_with_profile(sample, profile)
        } else {
            map_standard_sample(sample)
        }
    }

    fn find_matching_profile(&self, device_id: &str) -> Option<&crate::DeviceProfile> {
        let device = self
            .devices
            .iter()
            .find(|candidate| candidate.device_id == device_id);

        // profile 允许按硬件特征匹配，避免设备重连后 runtime device_id 变化导致配置失效。
        self.config
            .device_profiles
            .iter()
            .filter(|profile| profile_matches_device(profile, device, device_id))
            .max_by_key(|profile| profile.matcher.match_score())
    }

    fn track_activity(&mut self, sample: &RawDeviceSample) {
        let activity_score = sample.buttons.iter().copied().fold(0.0_f32, f32::max).max(
            sample
                .axes
                .iter()
                .map(|value| value.abs())
                .fold(0.0_f32, f32::max),
        );
        if activity_score > 0.15 {
            self.last_active_at
                .insert(sample.device_id.clone(), sample.observed_at_ms);
        }
    }

    fn clear_active_device_if_matches(&mut self, device_id: &str) {
        for slot in &mut self.active_device_by_pad {
            if slot.as_deref() == Some(device_id) {
                *slot = None;
            }
        }
    }

    fn active_device(&self, pad_id: LogicalPadId) -> Option<&str> {
        self.active_device_by_pad[pad_slot(pad_id)].as_deref()
    }

    fn set_active_device(&mut self, pad_id: LogicalPadId, device_id: Option<String>) {
        self.active_device_by_pad[pad_slot(pad_id)] = device_id;
    }
}

fn filter_candidate_ids(connected_ids: &[String], preferred_ids: &[String]) -> Vec<String> {
    if preferred_ids.is_empty() {
        return connected_ids.to_vec();
    }

    preferred_ids
        .iter()
        .filter(|device_id| {
            connected_ids
                .iter()
                .any(|connected_id| connected_id == *device_id)
        })
        .cloned()
        .collect()
}

fn profile_matches_device(
    profile: &crate::DeviceProfile,
    device: Option<&OhMyGamepadDeviceDto>,
    device_id: &str,
) -> bool {
    match device {
        Some(device) => profile.matcher.matches(device),
        None => matcher_can_match_without_metadata(&profile.matcher, device_id),
    }
}

fn matcher_can_match_without_metadata(
    matcher: &crate::DeviceProfileMatcher,
    device_id: &str,
) -> bool {
    if let Some(expected_device_id) = matcher.device_id.as_deref() {
        if expected_device_id != device_id {
            return false;
        }
    }

    matcher.vendor_id.is_none()
        && matcher.product_id.is_none()
        && matcher.backend.is_none()
        && matcher
            .name_contains
            .as_deref()
            .map(str::trim)
            .map(str::is_empty)
            .unwrap_or(true)
}

fn pad_slot(pad_id: LogicalPadId) -> usize {
    match pad_id {
        LogicalPadId::Pad0 => 0,
        LogicalPadId::Pad1 => 1,
        LogicalPadId::Pad2 => 2,
        LogicalPadId::Pad3 => 3,
    }
}

fn pad_order(pad_id: LogicalPadId) -> u8 {
    pad_slot(pad_id) as u8
}

fn pad_payload_changed(left: &LogicalPadSnapshotDto, right: &LogicalPadSnapshotDto) -> bool {
    left.device_ids != right.device_ids
        || left.state != right.state
        // 扩展键（背键等）可能只出现在 raw_buttons 中、不改变标准逻辑键位，必须参与去重判断，
        // 否则映射采集阶段收不到 slotSnapshot。
        || left.raw_buttons != right.raw_buttons
        // SDL prime 与轮询可能推送与上一帧完全相同的轴/键向量，但仍代表一次新的硬件观测；
        // 若不把 sampled_at_ms 纳入比较，逻辑层会长期不推进 sample_seq（首开/恢复后 UI 无输入）。
        || left.sampled_at_ms != right.sampled_at_ms
}

fn collect_pressed_raw_buttons(
    device_ids: &[String],
    raw_samples: &std::collections::HashMap<String, RawDeviceSample>,
) -> Option<Vec<ohmygamepad_protocol::GamepadRawButtonStateDto>> {
    const PRESS_THRESHOLD: f32 = 0.5;
    const MAX_BUTTONS: usize = 32;

    let mut merged: std::collections::HashMap<usize, f32> = std::collections::HashMap::new();
    for device_id in device_ids {
        let Some(sample) = raw_samples.get(device_id) else {
            continue;
        };
        for (index, value) in sample.buttons.iter().copied().enumerate() {
            if value <= PRESS_THRESHOLD {
                continue;
            }
            merged
                .entry(index)
                .and_modify(|existing| {
                    if value > *existing {
                        *existing = value;
                    }
                })
                .or_insert(value);
        }
    }

    if merged.is_empty() {
        return None;
    }

    let mut raw_buttons = merged
        .into_iter()
        .map(|(index, value)| ohmygamepad_protocol::GamepadRawButtonStateDto { index, value })
        .collect::<Vec<_>>();
    raw_buttons.sort_by(|a, b| {
        b.value
            .partial_cmp(&a.value)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.index.cmp(&b.index))
    });
    raw_buttons.truncate(MAX_BUTTONS);
    Some(raw_buttons)
}

fn hz_to_interval_ms(hz: u16) -> u64 {
    let normalized_hz = hz.max(1) as u64;
    (1_000 / normalized_hz).max(1)
}

#[cfg(test)]
#[path = "engine.test.rs"]
mod tests;
