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
}

impl Default for Sdl3BackendConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_events_per_poll: 256,
        }
    }
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
        let (source, _) = RealSdl3Source::new()?;
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
