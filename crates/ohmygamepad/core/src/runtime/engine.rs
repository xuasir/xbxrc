use std::collections::{HashMap, HashSet};

use ohmygamepad_protocol::{
    LogicalPadBindingDto, LogicalPadId, LogicalPadSnapshotDto, OhMyGamepadBindingModeDto,
    OhMyGamepadDeviceDto, OhMyGamepadRouteTargetDto, OhMyGamepadRuntimeHapticsDto,
    OhMyGamepadRuntimeSnapshotDto, OhMyGamepadSamplingConfigDto,
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
        }
    }

    pub fn config(&self) -> &InputCoreConfig {
        &self.config
    }

    pub fn runtime_snapshot(&self) -> OhMyGamepadRuntimeSnapshotDto {
        OhMyGamepadRuntimeSnapshotDto {
            devices: self.devices.clone(),
            bindings: self.config.bindings.clone(),
            route_target: self.config.route_target.clone(),
            sampling: self.config.sampling.clone(),
            pads: self.pads.clone(),
            haptics: OhMyGamepadRuntimeHapticsDto::default(),
        }
    }

    pub fn replace_sampling_config(&mut self, sampling: OhMyGamepadSamplingConfigDto) {
        self.config.sampling = sampling;
    }

    pub fn replace_route_target(&mut self, route_target: OhMyGamepadRouteTargetDto) {
        self.config.route_target = route_target;
        self.sample_once();
    }

    pub fn replace_bindings(&mut self, bindings: Vec<LogicalPadBindingDto>) {
        self.config.bindings = bindings;
        self.sample_once();
    }

    pub fn replace_device_profiles(&mut self, device_profiles: Vec<crate::DeviceProfile>) {
        self.config.device_profiles = device_profiles;
        self.sample_once();
    }

    pub fn tick(&mut self) {
        self.poll_backend();
        self.sample_once();
    }

    pub fn poll_backend(&mut self) {
        let poll = self.backend.poll();
        self.apply_backend_poll(poll);
    }

    pub fn sample_once(&mut self) {
        self.refresh_pad_snapshots();
    }

    pub fn push_pad_snapshot(&mut self, snapshot: LogicalPadSnapshotDto) {
        // 这里先固定按逻辑 pad 覆盖快照，后续再补采样/过滤流水线。
        if let Some(existing) = self
            .pads
            .iter_mut()
            .find(|current| current.pad_id == snapshot.pad_id)
        {
            *existing = snapshot.clone();
        } else {
            self.pads.push(snapshot.clone());
        }
        self.ui_sink.emit_pad_snapshot(&snapshot);
        self.stream_sink.emit_pad_snapshot(&snapshot);
    }

    pub fn pad_snapshot(&self, pad_id: LogicalPadId) -> Option<&LogicalPadSnapshotDto> {
        self.pads.iter().find(|snapshot| snapshot.pad_id == pad_id)
    }

    pub fn reset_state(&mut self) {
        // 清理原始采样，防止恢复时使用过期的硬件状态
        self.raw_samples.clear();
        self.last_active_at.clear();

        // 将所有逻辑手柄重置为中性状态
        for snapshot in &mut self.pads {
            snapshot.state = default_logical_pad_state();
            // 立即向接收端同步“按键全部弹起”的状态，防止 UI 粘滞。
            self.ui_sink.emit_pad_snapshot(snapshot);
            self.stream_sink.emit_pad_snapshot(snapshot);
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

    fn refresh_pad_snapshots(&mut self) {
        self.sample_seq = self.sample_seq.saturating_add(1);
        let bindings = self.effective_bindings();
        let mut next_pads = Vec::with_capacity(bindings.len());
        let mut reserved_split_devices = HashSet::new();

        for binding in &bindings {
            let selected_device_ids =
                self.resolve_binding_device_ids(binding, &mut reserved_split_devices);
            let snapshot = self.build_pad_snapshot(binding.pad_id, selected_device_ids);
            next_pads.push(snapshot);
        }

        next_pads.sort_by_key(|snapshot| pad_order(snapshot.pad_id));

        for snapshot in &next_pads {
            let changed = self
                .pads
                .iter()
                .find(|current| current.pad_id == snapshot.pad_id)
                .map(|current| pad_payload_changed(current, snapshot))
                .unwrap_or(true);
            if changed {
                self.ui_sink.emit_pad_snapshot(snapshot);
                self.stream_sink.emit_pad_snapshot(snapshot);
            }
        }

        self.pads = next_pads;
    }

    fn effective_bindings(&self) -> Vec<LogicalPadBindingDto> {
        let mut bindings = if self.config.bindings.is_empty() {
            vec![LogicalPadBindingDto {
                pad_id: LogicalPadId::Pad0,
                mode: OhMyGamepadBindingModeDto::SingleActive,
                device_ids: Vec::new(),
            }]
        } else {
            self.config.bindings.clone()
        };
        bindings.sort_by_key(|binding| pad_order(binding.pad_id));
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
                .resolve_single_active_device(binding.pad_id, &candidate_ids)
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
                .resolve_failover_device(binding.pad_id, &candidate_ids)
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
        &self,
        pad_id: LogicalPadId,
        device_ids: Vec<String>,
    ) -> LogicalPadSnapshotDto {
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

        LogicalPadSnapshotDto {
            pad_id,
            device_ids,
            sampled_at_ms,
            sample_seq: self.sample_seq,
            route_target: self.config.route_target.clone(),
            state: if states.is_empty() {
                default_logical_pad_state()
            } else {
                merge_states(&states)
            },
        }
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
        || left.route_target != right.route_target
        || left.state != right.state
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::VecDeque, rc::Rc};

    use ohmygamepad_protocol::{
        LogicalButtonsStateDto, OhMyGamepadBackendKindDto, OhMyGamepadBindingModeDto,
        OhMyGamepadCapabilityFlagsDto, OhMyGamepadDeviceDto,
    };

    use super::InputCore;
    use crate::{
        BackendPollResult, ButtonMapping, DeviceLifecycleEvent, DeviceProfile,
        DeviceProfileMatcher, FilterConfig, InputBackend, InputCoreConfig, RawDeviceSample,
        StreamSink, UiSink,
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
    struct SharedUiSink {
        devices: Rc<RefCell<Vec<Vec<OhMyGamepadDeviceDto>>>>,
        pads: Rc<RefCell<Vec<ohmygamepad_protocol::LogicalPadSnapshotDto>>>,
    }

    impl UiSink for SharedUiSink {
        fn emit_devices_changed(&mut self, devices: &[OhMyGamepadDeviceDto]) {
            self.devices.borrow_mut().push(devices.to_vec());
        }

        fn emit_pad_snapshot(&mut self, snapshot: &ohmygamepad_protocol::LogicalPadSnapshotDto) {
            self.pads.borrow_mut().push(snapshot.clone());
        }
    }

    #[derive(Default, Clone)]
    struct SharedStreamSink {
        pads: Rc<RefCell<Vec<ohmygamepad_protocol::LogicalPadSnapshotDto>>>,
    }

    impl StreamSink for SharedStreamSink {
        fn emit_pad_snapshot(&mut self, snapshot: &ohmygamepad_protocol::LogicalPadSnapshotDto) {
            self.pads.borrow_mut().push(snapshot.clone());
        }
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

    fn sample(
        device_id: &str,
        observed_at_ms: u64,
        buttons: Vec<f32>,
        axes: Vec<f32>,
    ) -> RawDeviceSample {
        RawDeviceSample {
            device_id: device_id.to_owned(),
            observed_at_ms,
            buttons,
            axes,
        }
    }

    #[test]
    fn default_single_active_binding_selects_latest_active_device() {
        let ui_sink = SharedUiSink::default();
        let ui_pads = ui_sink.pads.clone();
        let backend = ScriptedBackend::new(vec![BackendPollResult {
            device_events: vec![
                DeviceLifecycleEvent::Added(device("pad-a")),
                DeviceLifecycleEvent::Added(device("pad-b")),
            ],
            samples: vec![
                sample("pad-a", 10, vec![1.0], vec![0.0, 0.0, 0.0, 0.0]),
                sample(
                    "pad-b",
                    20,
                    vec![0.0, 0.0, 0.0, 1.0],
                    vec![0.0, 0.0, 0.0, 0.0],
                ),
            ],
        }]);
        let mut core = InputCore::new(
            InputCoreConfig::default(),
            backend,
            ui_sink,
            SharedStreamSink::default(),
        );

        core.tick();

        let pads = ui_pads.borrow();
        let latest = pads.last().expect("expected pad snapshot");
        assert_eq!(latest.pad_id, ohmygamepad_protocol::LogicalPadId::Pad0);
        assert_eq!(latest.device_ids, vec!["pad-b".to_owned()]);
        assert_eq!(latest.state.buttons.north, 1.0);
    }

    #[test]
    fn fixed_device_binding_ignores_other_devices() {
        let ui_sink = SharedUiSink::default();
        let ui_pads = ui_sink.pads.clone();
        let backend = ScriptedBackend::new(vec![BackendPollResult {
            device_events: vec![
                DeviceLifecycleEvent::Added(device("pad-a")),
                DeviceLifecycleEvent::Added(device("pad-b")),
            ],
            samples: vec![
                sample("pad-a", 10, vec![1.0], vec![0.0, 0.0, 0.0, 0.0]),
                sample("pad-b", 20, vec![0.0, 1.0], vec![0.0, 0.0, 0.0, 0.0]),
            ],
        }]);
        let mut config = InputCoreConfig::default();
        config.bindings = vec![ohmygamepad_protocol::LogicalPadBindingDto {
            pad_id: ohmygamepad_protocol::LogicalPadId::Pad0,
            mode: OhMyGamepadBindingModeDto::FixedDevice,
            device_ids: vec!["pad-a".to_owned()],
        }];
        let mut core = InputCore::new(config, backend, ui_sink, SharedStreamSink::default());

        core.tick();

        let pads = ui_pads.borrow();
        let latest = pads.last().expect("expected pad snapshot");
        assert_eq!(latest.device_ids, vec!["pad-a".to_owned()]);
        assert_eq!(latest.state.buttons.south, 1.0);
        assert_eq!(latest.state.buttons.east, 0.0);
    }

    #[test]
    fn merged_binding_combines_buttons_and_axes() {
        let ui_sink = SharedUiSink::default();
        let ui_pads = ui_sink.pads.clone();
        let backend = ScriptedBackend::new(vec![BackendPollResult {
            device_events: vec![
                DeviceLifecycleEvent::Added(device("pad-a")),
                DeviceLifecycleEvent::Added(device("pad-b")),
            ],
            samples: vec![
                sample("pad-a", 10, vec![1.0], vec![0.6, 0.0, 0.0, 0.0]),
                sample(
                    "pad-b",
                    20,
                    vec![0.0, 0.0, 0.0, 1.0],
                    vec![-0.2, 0.0, 0.0, 0.7],
                ),
            ],
        }]);
        let mut config = InputCoreConfig::default();
        config.bindings = vec![ohmygamepad_protocol::LogicalPadBindingDto {
            pad_id: ohmygamepad_protocol::LogicalPadId::Pad0,
            mode: OhMyGamepadBindingModeDto::Merged,
            device_ids: vec!["pad-a".to_owned(), "pad-b".to_owned()],
        }];
        let mut core = InputCore::new(config, backend, ui_sink, SharedStreamSink::default());

        core.tick();

        let pads = ui_pads.borrow();
        let latest = pads.last().expect("expected pad snapshot");
        assert_eq!(latest.state.buttons.south, 1.0);
        assert_eq!(latest.state.buttons.north, 1.0);
        assert!(latest.state.left_stick.x > 0.5);
        assert!(latest.state.right_stick.y > 0.6);
    }

    #[test]
    fn split_binding_assigns_unique_devices_per_pad() {
        let ui_sink = SharedUiSink::default();
        let ui_pads = ui_sink.pads.clone();
        let backend = ScriptedBackend::new(vec![BackendPollResult {
            device_events: vec![
                DeviceLifecycleEvent::Added(device("pad-a")),
                DeviceLifecycleEvent::Added(device("pad-b")),
            ],
            samples: vec![
                sample("pad-a", 10, vec![1.0], vec![0.0, 0.0, 0.0, 0.0]),
                sample("pad-b", 20, vec![0.0, 1.0], vec![0.0, 0.0, 0.0, 0.0]),
            ],
        }]);
        let mut config = InputCoreConfig::default();
        config.bindings = vec![
            ohmygamepad_protocol::LogicalPadBindingDto {
                pad_id: ohmygamepad_protocol::LogicalPadId::Pad0,
                mode: OhMyGamepadBindingModeDto::Split,
                device_ids: Vec::new(),
            },
            ohmygamepad_protocol::LogicalPadBindingDto {
                pad_id: ohmygamepad_protocol::LogicalPadId::Pad1,
                mode: OhMyGamepadBindingModeDto::Split,
                device_ids: Vec::new(),
            },
        ];
        let mut core = InputCore::new(config, backend, ui_sink, SharedStreamSink::default());

        core.tick();

        let pads = ui_pads.borrow();
        assert_eq!(pads.len(), 2);
        assert_ne!(pads[0].device_ids, pads[1].device_ids);
    }

    #[test]
    fn unchanged_snapshot_is_not_emitted_twice() {
        let ui_sink = SharedUiSink::default();
        let ui_pads = ui_sink.pads.clone();
        let backend = ScriptedBackend::new(vec![
            BackendPollResult {
                device_events: vec![DeviceLifecycleEvent::Added(device("pad-a"))],
                samples: vec![sample("pad-a", 10, vec![1.0], vec![0.0, 0.0, 0.0, 0.0])],
            },
            BackendPollResult {
                device_events: vec![],
                samples: vec![sample("pad-a", 11, vec![1.0], vec![0.0, 0.0, 0.0, 0.0])],
            },
        ]);
        let mut core = InputCore::new(
            InputCoreConfig::default(),
            backend,
            ui_sink,
            SharedStreamSink::default(),
        );

        core.tick();
        core.tick();

        let pads = ui_pads.borrow();
        assert_eq!(pads.len(), 1);
        assert_eq!(
            pads[0].state.buttons,
            LogicalButtonsStateDto {
                south: 1.0,
                ..LogicalButtonsStateDto::default()
            }
        );
    }

    #[test]
    fn last_active_failover_keeps_current_device_until_disconnect() {
        let ui_sink = SharedUiSink::default();
        let ui_pads = ui_sink.pads.clone();
        let backend = ScriptedBackend::new(vec![
            BackendPollResult {
                device_events: vec![
                    DeviceLifecycleEvent::Added(device("pad-a")),
                    DeviceLifecycleEvent::Added(device("pad-b")),
                ],
                samples: vec![
                    sample("pad-a", 20, vec![1.0], vec![0.0, 0.0, 0.0, 0.0]),
                    sample("pad-b", 10, vec![0.0, 1.0], vec![0.0, 0.0, 0.0, 0.0]),
                ],
            },
            BackendPollResult {
                device_events: vec![],
                samples: vec![sample(
                    "pad-b",
                    30,
                    vec![0.0, 1.0],
                    vec![0.0, 0.0, 0.0, 0.0],
                )],
            },
            BackendPollResult {
                device_events: vec![DeviceLifecycleEvent::Removed {
                    device_id: "pad-a".to_owned(),
                    observed_at_ms: 40,
                }],
                samples: vec![sample(
                    "pad-b",
                    40,
                    vec![0.0, 1.0],
                    vec![0.0, 0.0, 0.0, 0.0],
                )],
            },
        ]);
        let mut config = InputCoreConfig::default();
        config.bindings = vec![ohmygamepad_protocol::LogicalPadBindingDto {
            pad_id: ohmygamepad_protocol::LogicalPadId::Pad0,
            mode: OhMyGamepadBindingModeDto::LastActiveFailover,
            device_ids: Vec::new(),
        }];
        let mut core = InputCore::new(config, backend, ui_sink, SharedStreamSink::default());

        core.tick();
        core.tick();
        core.tick();

        let pads = ui_pads.borrow();
        assert_eq!(pads[0].device_ids, vec!["pad-a".to_owned()]);
        assert_eq!(pads[1].device_ids, vec!["pad-b".to_owned()]);
    }

    #[test]
    fn route_target_change_emits_new_snapshot() {
        let ui_sink = SharedUiSink::default();
        let ui_pads = ui_sink.pads.clone();
        let backend = ScriptedBackend::new(vec![BackendPollResult {
            device_events: vec![DeviceLifecycleEvent::Added(device("pad-a"))],
            samples: vec![sample("pad-a", 10, vec![1.0], vec![0.0, 0.0, 0.0, 0.0])],
        }]);
        let mut core = InputCore::new(
            InputCoreConfig::default(),
            backend,
            ui_sink,
            SharedStreamSink::default(),
        );

        core.tick();
        core.replace_route_target(
            ohmygamepad_protocol::OhMyGamepadRouteTargetDto::StreamSession {
                session_id: "session-1".to_owned(),
            },
        );

        let pads = ui_pads.borrow();
        assert_eq!(pads.len(), 2);
        assert_eq!(
            pads[1].route_target,
            ohmygamepad_protocol::OhMyGamepadRouteTargetDto::StreamSession {
                session_id: "session-1".to_owned()
            }
        );
    }

    #[test]
    fn custom_device_profile_is_applied_during_snapshot_build() {
        let ui_sink = SharedUiSink::default();
        let ui_pads = ui_sink.pads.clone();
        let backend = ScriptedBackend::new(vec![BackendPollResult {
            device_events: vec![DeviceLifecycleEvent::Added(device("pad-a"))],
            samples: vec![sample(
                "pad-a",
                10,
                vec![0.0, 0.0, 0.0, 1.0],
                vec![0.05, 0.0, 0.0, 0.0],
            )],
        }]);
        let mut config = InputCoreConfig::default();
        config.device_profiles = vec![DeviceProfile {
            matcher: DeviceProfileMatcher {
                device_id: Some("pad-a".to_owned()),
                ..DeviceProfileMatcher::default()
            },
            buttons: ButtonMapping {
                south: 3,
                north: 0,
                ..ButtonMapping::default()
            },
            filter: FilterConfig {
                stick_deadzone: 0.0,
                stick_epsilon: 0.0,
                trigger_deadzone: 0.03,
                trigger_epsilon: 0.01,
                button_epsilon: 0.0001,
            },
            ..DeviceProfile::default()
        }];
        let mut core = InputCore::new(config, backend, ui_sink, SharedStreamSink::default());

        core.tick();

        let pads = ui_pads.borrow();
        let latest = pads.last().expect("expected pad snapshot");
        assert_eq!(latest.state.buttons.south, 1.0);
        assert!(latest.state.left_stick.x > 0.04);
    }

    #[test]
    fn hardware_profile_survives_runtime_device_id_reconnect() {
        let ui_sink = SharedUiSink::default();
        let ui_pads = ui_sink.pads.clone();

        let mut first_device = device("pad-a");
        first_device.name = "Xbox Wireless Controller".to_owned();
        first_device.backend = Some(OhMyGamepadBackendKindDto::Gilrs);
        first_device.vendor_id = Some(0x045e);
        first_device.product_id = Some(0x0b13);

        let mut second_device = device("pad-a-reconnected");
        second_device.name = "Xbox Wireless Controller".to_owned();
        second_device.backend = Some(OhMyGamepadBackendKindDto::Gilrs);
        second_device.vendor_id = Some(0x045e);
        second_device.product_id = Some(0x0b13);

        let backend = ScriptedBackend::new(vec![
            BackendPollResult {
                device_events: vec![DeviceLifecycleEvent::Added(first_device)],
                samples: vec![sample(
                    "pad-a",
                    10,
                    vec![0.0, 0.0, 0.0, 1.0],
                    vec![0.0, 0.0, 0.0, 0.0],
                )],
            },
            BackendPollResult {
                device_events: vec![
                    DeviceLifecycleEvent::Removed {
                        device_id: "pad-a".to_owned(),
                        observed_at_ms: 20,
                    },
                    DeviceLifecycleEvent::Added(second_device),
                ],
                samples: vec![sample(
                    "pad-a-reconnected",
                    20,
                    vec![0.0, 0.0, 0.0, 1.0],
                    vec![0.0, 0.0, 0.0, 0.0],
                )],
            },
        ]);

        let mut config = InputCoreConfig::default();
        config.device_profiles = vec![DeviceProfile {
            matcher: DeviceProfileMatcher {
                vendor_id: Some(0x045e),
                product_id: Some(0x0b13),
                backend: Some(OhMyGamepadBackendKindDto::Gilrs),
                name_contains: Some("wireless".to_owned()),
                ..DeviceProfileMatcher::default()
            },
            buttons: ButtonMapping {
                south: 3,
                ..ButtonMapping::default()
            },
            ..DeviceProfile::default()
        }];
        let mut core = InputCore::new(config, backend, ui_sink, SharedStreamSink::default());

        core.tick();
        core.tick();

        let pads = ui_pads.borrow();
        let latest = pads.last().expect("expected pad snapshot after reconnect");
        assert_eq!(latest.device_ids, vec!["pad-a-reconnected".to_owned()]);
        assert_eq!(latest.state.buttons.south, 1.0);
    }

    #[test]
    fn more_specific_device_id_profile_overrides_hardware_profile() {
        let ui_sink = SharedUiSink::default();
        let ui_pads = ui_sink.pads.clone();

        let mut matched_device = device("pad-a");
        matched_device.name = "Xbox Wireless Controller".to_owned();
        matched_device.backend = Some(OhMyGamepadBackendKindDto::Gilrs);
        matched_device.vendor_id = Some(0x045e);
        matched_device.product_id = Some(0x0b13);

        let backend = ScriptedBackend::new(vec![BackendPollResult {
            device_events: vec![DeviceLifecycleEvent::Added(matched_device)],
            samples: vec![sample(
                "pad-a",
                10,
                vec![0.0, 0.0, 1.0, 0.0],
                vec![0.0, 0.0, 0.0, 0.0],
            )],
        }]);

        let mut config = InputCoreConfig::default();
        config.device_profiles = vec![
            DeviceProfile {
                matcher: DeviceProfileMatcher {
                    vendor_id: Some(0x045e),
                    product_id: Some(0x0b13),
                    backend: Some(OhMyGamepadBackendKindDto::Gilrs),
                    ..DeviceProfileMatcher::default()
                },
                buttons: ButtonMapping {
                    south: 3,
                    ..ButtonMapping::default()
                },
                ..DeviceProfile::default()
            },
            DeviceProfile {
                matcher: DeviceProfileMatcher {
                    device_id: Some("pad-a".to_owned()),
                    ..DeviceProfileMatcher::default()
                },
                buttons: ButtonMapping {
                    south: 2,
                    ..ButtonMapping::default()
                },
                ..DeviceProfile::default()
            },
        ];
        let mut core = InputCore::new(config, backend, ui_sink, SharedStreamSink::default());

        core.tick();

        let pads = ui_pads.borrow();
        let latest = pads.last().expect("expected pad snapshot");
        assert_eq!(latest.state.buttons.south, 1.0);
    }
}
