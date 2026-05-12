use std::path::PathBuf;

use std::collections::{hash_map::Entry, HashMap};

use ohmygamepad_core::{BackendPollResult, DeviceLifecycleEvent, InputBackend, RawDeviceSample};
use ohmygamepad_protocol::OhMyGamepadDeviceDto;

use crate::{
    NoopSdl3Source, RealSdl3Source, Sdl3DeviceDescriptor, Sdl3InputEvent, Sdl3InputEventKind,
    Sdl3Source, Sdl3SourceInitError,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sdl3BackendConfig {
    pub enabled: bool,
    pub max_events_per_poll: usize,
    pub mapping_paths: Vec<PathBuf>,
    pub extra_mappings: Vec<String>,
    pub ignored_device_guids: Vec<String>,
    pub ignored_vid_pids: Vec<(u16, u16)>,
    pub ignored_name_contains: Vec<String>,
}

impl Default for Sdl3BackendConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_events_per_poll: 256,
            mapping_paths: default_mapping_paths(),
            extra_mappings: parse_multiline_env("OHMYGAMEPAD_SDL3_EXTRA_MAPPINGS"),
            ignored_device_guids: parse_csv_env("OHMYGAMEPAD_SDL3_IGNORE_GUIDS"),
            ignored_vid_pids: parse_vid_pid_env("OHMYGAMEPAD_SDL3_IGNORE_VID_PIDS"),
            ignored_name_contains: parse_csv_env("OHMYGAMEPAD_SDL3_IGNORE_NAME_CONTAINS"),
        }
    }
}

fn default_mapping_paths() -> Vec<PathBuf> {
    [
        PathBuf::from("src-tauri/resources/gamecontrollerdb.txt"),
        PathBuf::from("resources/gamecontrollerdb.txt"),
        PathBuf::from("gamecontrollerdb.txt"),
    ]
    .into_iter()
    .filter(|path| path.is_file())
    .collect()
}

fn parse_csv_env(key: &str) -> Vec<String> {
    std::env::var(key)
        .ok()
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_multiline_env(key: &str) -> Vec<String> {
    std::env::var(key)
        .ok()
        .map(|value| {
            value
                .lines()
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_vid_pid_env(key: &str) -> Vec<(u16, u16)> {
    parse_csv_env(key)
        .into_iter()
        .filter_map(|item| {
            let (vendor, product) = item.split_once(':')?;
            let vendor = u16::from_str_radix(vendor.trim(), 16).ok()?;
            let product = u16::from_str_radix(product.trim(), 16).ok()?;
            Some((vendor, product))
        })
        .collect()
}

#[derive(Debug)]
struct TrackedDevice {
    descriptor: Sdl3DeviceDescriptor,
    announced_at_ms: u64,
    sample_observed_at_ms: u64,
    buttons: Vec<f32>,
    axes: Vec<f32>,
    dirty_sample: bool,
}

impl TrackedDevice {
    fn new(descriptor: Sdl3DeviceDescriptor, observed_at_ms: u64) -> Self {
        Self {
            descriptor,
            announced_at_ms: observed_at_ms,
            sample_observed_at_ms: observed_at_ms,
            buttons: Vec::new(),
            axes: Vec::new(),
            dirty_sample: false,
        }
    }

    fn device(&self) -> OhMyGamepadDeviceDto {
        self.descriptor.to_connected_device(self.announced_at_ms)
    }

    fn update_descriptor(&mut self, descriptor: Sdl3DeviceDescriptor, observed_at_ms: u64) -> bool {
        let changed = self.descriptor != descriptor;
        self.descriptor = descriptor;
        self.announced_at_ms = observed_at_ms;
        self.sample_observed_at_ms = self.sample_observed_at_ms.max(observed_at_ms);
        changed
    }

    fn update_button(&mut self, index: usize, value: f32, observed_at_ms: u64) {
        if index >= self.buttons.len() {
            self.buttons.resize(index + 1, 0.0);
        }
        if (self.buttons[index] - value).abs() <= f32::EPSILON {
            return;
        }

        self.buttons[index] = value;
        self.sample_observed_at_ms = observed_at_ms;
        self.dirty_sample = true;
    }

    fn update_axis(&mut self, index: usize, value: f32, observed_at_ms: u64) {
        if index >= self.axes.len() {
            self.axes.resize(index + 1, 0.0);
        }
        if (self.axes[index] - value).abs() <= f32::EPSILON {
            return;
        }

        self.axes[index] = value;
        self.sample_observed_at_ms = observed_at_ms;
        self.dirty_sample = true;
    }

    fn sync_snapshot(
        &mut self,
        buttons: Vec<f32>,
        axes: Vec<f32>,
        observed_at_ms: u64,
        force_dirty: bool,
    ) {
        let first_snapshot = self.buttons.is_empty() && self.axes.is_empty();
        let changed = first_snapshot || self.buttons != buttons || self.axes != axes;
        self.buttons = buttons;
        self.axes = axes;
        self.sample_observed_at_ms = observed_at_ms;
        // 同一轮 backend poll 里可能先收到“建立基线”的 Snapshot，
        // 随后又收到多条内容完全相同的轮询 Snapshot。这里一旦本轮已标脏，
        // 后续相同快照不能把 dirty 再冲掉，否则 runtime 会错过首条 raw sample。
        self.dirty_sample |= changed || force_dirty;
    }

    fn take_sample(&mut self) -> Option<RawDeviceSample> {
        if !self.dirty_sample {
            return None;
        }

        self.dirty_sample = false;
        Some(RawDeviceSample {
            device_id: self.descriptor.device_id.clone(),
            observed_at_ms: self.sample_observed_at_ms,
            buttons: self.buttons.clone(),
            axes: self.axes.clone(),
        })
    }
}

#[derive(Debug)]
pub struct Sdl3Backend<TSource = NoopSdl3Source> {
    config: Sdl3BackendConfig,
    source: TSource,
    devices: HashMap<String, TrackedDevice>,
}

impl Sdl3Backend<RealSdl3Source> {
    pub fn new(config: Sdl3BackendConfig) -> Result<Self, Sdl3SourceInitError> {
        let (source, _) = RealSdl3Source::new(config.clone())?;
        Ok(Self::with_source(config, source))
    }
}

impl Sdl3Backend<NoopSdl3Source> {
    pub fn new_noop(config: Sdl3BackendConfig) -> Self {
        Self::with_source(config, NoopSdl3Source)
    }
}

impl<TSource> Sdl3Backend<TSource> {
    pub fn config(&self) -> &Sdl3BackendConfig {
        &self.config
    }
}

impl<TSource> Sdl3Backend<TSource>
where
    TSource: Sdl3Source,
{
    pub fn with_source(config: Sdl3BackendConfig, source: TSource) -> Self {
        Self {
            config,
            source,
            devices: HashMap::new(),
        }
    }

    fn max_events_per_poll(&self) -> usize {
        self.config.max_events_per_poll.max(1)
    }

    fn ensure_device(
        &mut self,
        descriptor: Sdl3DeviceDescriptor,
        observed_at_ms: u64,
        device_events: &mut Vec<DeviceLifecycleEvent>,
    ) -> &mut TrackedDevice {
        match self.devices.entry(descriptor.device_id.clone()) {
            Entry::Occupied(entry) => {
                let device = entry.into_mut();
                if device.update_descriptor(descriptor, observed_at_ms) {
                    device_events.push(DeviceLifecycleEvent::Updated(device.device()));
                }
                device
            }
            Entry::Vacant(entry) => {
                let device = TrackedDevice::new(descriptor, observed_at_ms);
                device_events.push(DeviceLifecycleEvent::Added(device.device()));
                entry.insert(device)
            }
        }
    }

    fn handle_event(
        &mut self,
        event: Sdl3InputEvent,
        device_events: &mut Vec<DeviceLifecycleEvent>,
        samples: &mut Vec<RawDeviceSample>,
    ) {
        match event.kind {
            Sdl3InputEventKind::Connected => {
                let _ = self.ensure_device(event.device, event.observed_at_ms, device_events);
            }
            Sdl3InputEventKind::Disconnected => {
                if let Some(mut device) = self.devices.remove(&event.device.device_id) {
                    if let Some(sample) = device.take_sample() {
                        samples.push(sample);
                    }
                    device_events.push(DeviceLifecycleEvent::Removed {
                        device_id: event.device.device_id,
                        observed_at_ms: event.observed_at_ms,
                    });
                }
            }
            Sdl3InputEventKind::Snapshot { buttons, axes } => {
                let device = self.ensure_device(event.device, event.observed_at_ms, device_events);
                device.sync_snapshot(buttons, axes, event.observed_at_ms, false);
            }
            Sdl3InputEventKind::PrimeSnapshot { buttons, axes } => {
                let device = self.ensure_device(event.device, event.observed_at_ms, device_events);
                device.sync_snapshot(buttons, axes, event.observed_at_ms, true);
            }
            Sdl3InputEventKind::ButtonChanged { index, value } => {
                let device = self.ensure_device(event.device, event.observed_at_ms, device_events);
                device.update_button(index, value, event.observed_at_ms);
            }
            Sdl3InputEventKind::AxisChanged { index, value } => {
                let device = self.ensure_device(event.device, event.observed_at_ms, device_events);
                device.update_axis(index, value, event.observed_at_ms);
            }
            Sdl3InputEventKind::Dropped => {}
        }
    }

    fn collect_samples(&mut self) -> Vec<RawDeviceSample> {
        let mut device_ids = self.devices.keys().cloned().collect::<Vec<_>>();
        device_ids.sort();

        let mut samples = Vec::new();
        for device_id in device_ids {
            let Some(device) = self.devices.get_mut(&device_id) else {
                continue;
            };
            if let Some(sample) = device.take_sample() {
                samples.push(sample);
            }
        }
        samples
    }
}

impl<TSource> InputBackend for Sdl3Backend<TSource>
where
    TSource: Sdl3Source,
{
    fn poll(&mut self) -> BackendPollResult {
        if !self.config.enabled {
            return BackendPollResult::default();
        }

        let mut device_events = Vec::new();
        let mut samples = Vec::new();

        for _ in 0..self.max_events_per_poll() {
            let Some(event) = self.source.next_event() else {
                break;
            };
            self.handle_event(event, &mut device_events, &mut samples);
        }

        samples.extend(self.collect_samples());

        BackendPollResult {
            device_events,
            samples,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use ohmygamepad_core::InputBackend;

    use super::{Sdl3Backend, Sdl3BackendConfig};
    use crate::{Sdl3DeviceDescriptor, Sdl3InputEvent, Sdl3InputEventKind, Sdl3Source};

    struct QueueSource {
        events: VecDeque<Sdl3InputEvent>,
    }

    impl QueueSource {
        fn new(events: Vec<Sdl3InputEvent>) -> Self {
            Self {
                events: events.into(),
            }
        }
    }

    impl Sdl3Source for QueueSource {
        fn next_event(&mut self) -> Option<Sdl3InputEvent> {
            self.events.pop_front()
        }
    }

    #[test]
    fn stable_snapshots_do_not_clear_initial_dirty_sample() {
        let device = Sdl3DeviceDescriptor {
            device_id: "pad-1".to_owned(),
            name: "Test Pad".to_owned(),
            ..Sdl3DeviceDescriptor::default()
        };
        let snapshot_buttons = vec![0.0; 17];
        let snapshot_axes = vec![0.0, 0.0, 0.0, 0.0, -1.0, -1.0];
        let events = vec![
            Sdl3InputEvent {
                device: device.clone(),
                observed_at_ms: 10,
                kind: Sdl3InputEventKind::Connected,
            },
            Sdl3InputEvent {
                device: device.clone(),
                observed_at_ms: 11,
                kind: Sdl3InputEventKind::Snapshot {
                    buttons: snapshot_buttons.clone(),
                    axes: snapshot_axes.clone(),
                },
            },
            Sdl3InputEvent {
                device: device.clone(),
                observed_at_ms: 12,
                kind: Sdl3InputEventKind::Snapshot {
                    buttons: snapshot_buttons,
                    axes: snapshot_axes,
                },
            },
        ];

        let mut backend =
            Sdl3Backend::with_source(Sdl3BackendConfig::default(), QueueSource::new(events));
        let poll = backend.poll();

        assert_eq!(poll.device_events.len(), 1);
        assert_eq!(poll.samples.len(), 1);
        assert_eq!(poll.samples[0].device_id, "pad-1");
        assert_eq!(poll.samples[0].observed_at_ms, 12);
        assert_eq!(poll.samples[0].axes[4], -1.0);
        assert_eq!(poll.samples[0].axes[5], -1.0);
    }

    #[test]
    fn prime_snapshot_forces_backend_sample_when_vectors_unchanged() {
        let device = Sdl3DeviceDescriptor {
            device_id: "pad-prime".to_owned(),
            name: "Test Pad Prime".to_owned(),
            ..Sdl3DeviceDescriptor::default()
        };
        let snapshot_buttons = vec![0.0; 17];
        let snapshot_axes = vec![0.0_f32; 6];
        let events = vec![
            Sdl3InputEvent {
                device: device.clone(),
                observed_at_ms: 10,
                kind: Sdl3InputEventKind::Connected,
            },
            Sdl3InputEvent {
                device: device.clone(),
                observed_at_ms: 11,
                kind: Sdl3InputEventKind::Snapshot {
                    buttons: snapshot_buttons.clone(),
                    axes: snapshot_axes.clone(),
                },
            },
            Sdl3InputEvent {
                device: device.clone(),
                observed_at_ms: 12,
                kind: Sdl3InputEventKind::Snapshot {
                    buttons: snapshot_buttons.clone(),
                    axes: snapshot_axes.clone(),
                },
            },
            Sdl3InputEvent {
                device: device.clone(),
                observed_at_ms: 13,
                kind: Sdl3InputEventKind::PrimeSnapshot {
                    buttons: snapshot_buttons,
                    axes: snapshot_axes,
                },
            },
        ];

        let mut config = Sdl3BackendConfig::default();
        config.max_events_per_poll = 2;
        let mut backend = Sdl3Backend::with_source(config, QueueSource::new(events));
        let first = backend.poll();
        assert_eq!(first.samples.len(), 1);
        let second = backend.poll();
        assert_eq!(second.samples.len(), 1);
        assert_eq!(second.samples[0].observed_at_ms, 13);
        assert_eq!(second.samples[0].device_id, "pad-prime");
    }
}
