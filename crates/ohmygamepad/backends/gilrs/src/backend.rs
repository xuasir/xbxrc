use std::collections::{hash_map::Entry, HashMap};

use ohmygamepad_core::{BackendPollResult, DeviceLifecycleEvent, InputBackend, RawDeviceSample};
use ohmygamepad_protocol::OhMyGamepadDeviceDto;

use crate::{
    GilrsDeviceDescriptor, GilrsInputEvent, GilrsInputEventKind, GilrsSource, GilrsSourceInitError,
    NoopGilrsSource, RealGilrsSource,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GilrsBackendConfig {
    pub enabled: bool,
    pub max_events_per_poll: usize,
}

impl Default for GilrsBackendConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_events_per_poll: 256,
        }
    }
}

#[derive(Debug)]
struct TrackedDevice {
    descriptor: GilrsDeviceDescriptor,
    announced_at_ms: u64,
    sample_observed_at_ms: u64,
    buttons: Vec<f32>,
    axes: Vec<f32>,
    dirty_sample: bool,
}

impl TrackedDevice {
    fn new(descriptor: GilrsDeviceDescriptor, observed_at_ms: u64) -> Self {
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

    fn update_descriptor(
        &mut self,
        descriptor: GilrsDeviceDescriptor,
        observed_at_ms: u64,
    ) -> bool {
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

/**
 * gilrs 后端拆成“真实事件源 + 状态聚合器”两层。
 * 这样后续即使切换平台细节或补 mock/source，也不会影响 ohmygamepad-core 的消费接口。
 */
#[derive(Debug)]
pub struct GilrsBackend<TSource = NoopGilrsSource> {
    config: GilrsBackendConfig,
    source: TSource,
    devices: HashMap<String, TrackedDevice>,
}

pub type InputBackendAggregator<TSource = NoopGilrsSource> = GilrsBackend<TSource>;

impl GilrsBackend<RealGilrsSource> {
    pub fn new(config: GilrsBackendConfig) -> Result<Self, GilrsSourceInitError> {
        let (source, _) = RealGilrsSource::new()?;
        Ok(Self::with_source(config, source))
    }
}

impl GilrsBackend<NoopGilrsSource> {
    pub fn new_noop(config: GilrsBackendConfig) -> Self {
        Self::with_source(config, NoopGilrsSource)
    }
}

impl<TSource> GilrsBackend<TSource> {
    pub fn config(&self) -> &GilrsBackendConfig {
        &self.config
    }
}

impl<TSource> GilrsBackend<TSource>
where
    TSource: GilrsSource,
{
    pub fn with_source(config: GilrsBackendConfig, source: TSource) -> Self {
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
        descriptor: GilrsDeviceDescriptor,
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
        event: GilrsInputEvent,
        device_events: &mut Vec<DeviceLifecycleEvent>,
        samples: &mut Vec<RawDeviceSample>,
    ) {
        match event.kind {
            GilrsInputEventKind::Connected => {
                let _ = self.ensure_device(event.device, event.observed_at_ms, device_events);
            }
            GilrsInputEventKind::Disconnected => {
                if let Some(mut device) = self.devices.remove(&event.device.device_id) {
                    // 断连前先冲刷脏 sample，避免最后一帧输入在同轮 poll 中被吞掉。
                    if let Some(sample) = device.take_sample() {
                        samples.push(sample);
                    }
                    device_events.push(DeviceLifecycleEvent::Removed {
                        device_id: event.device.device_id,
                        observed_at_ms: event.observed_at_ms,
                    });
                }
            }
            GilrsInputEventKind::ButtonChanged { index, value } => {
                let device = self.ensure_device(event.device, event.observed_at_ms, device_events);
                device.update_button(index, value, event.observed_at_ms);
            }
            GilrsInputEventKind::AxisChanged { index, value } => {
                let device = self.ensure_device(event.device, event.observed_at_ms, device_events);
                device.update_axis(index, value, event.observed_at_ms);
            }
            GilrsInputEventKind::Dropped => {}
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

impl<TSource> InputBackend for GilrsBackend<TSource>
where
    TSource: GilrsSource,
{
    fn poll(&mut self) -> BackendPollResult {
        if !self.config.enabled {
            return BackendPollResult::default();
        }

        let mut device_events = Vec::new();
        let mut samples = Vec::new();

        // 每次 poll 只消费有限事件，避免输入线程被单次爆量事件拖慢。
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

    use ohmygamepad_protocol::{
        OhMyGamepadBackendKindDto, OhMyGamepadCapabilityFlagsDto, OhMyGamepadConnectionKindDto,
    };

    use super::{GilrsBackend, GilrsBackendConfig};
    use crate::{GilrsDeviceDescriptor, GilrsInputEvent, GilrsInputEventKind, GilrsSource};
    use ohmygamepad_core::InputBackend;

    #[derive(Debug, Default)]
    struct ScriptedGilrsSource {
        events: VecDeque<GilrsInputEvent>,
    }

    impl ScriptedGilrsSource {
        fn new(events: Vec<GilrsInputEvent>) -> Self {
            Self {
                events: VecDeque::from(events),
            }
        }
    }

    impl GilrsSource for ScriptedGilrsSource {
        fn next_event(&mut self) -> Option<GilrsInputEvent> {
            self.events.pop_front()
        }
    }

    fn descriptor(device_id: &str) -> GilrsDeviceDescriptor {
        GilrsDeviceDescriptor {
            device_id: device_id.to_owned(),
            name: "Xbox Wireless Controller".to_owned(),
            connection: Some(OhMyGamepadConnectionKindDto::Bluetooth),
            vendor_id: Some(0x045e),
            product_id: Some(0x0b13),
            capabilities: OhMyGamepadCapabilityFlagsDto {
                basic_rumble: true,
                advanced_haptics: false,
                battery: true,
            },
        }
    }

    fn event(device_id: &str, observed_at_ms: u64, kind: GilrsInputEventKind) -> GilrsInputEvent {
        GilrsInputEvent {
            device: descriptor(device_id),
            observed_at_ms,
            kind,
        }
    }

    #[test]
    fn poll_emits_added_device_and_sample() {
        let mut backend = GilrsBackend::with_source(
            GilrsBackendConfig::default(),
            ScriptedGilrsSource::new(vec![
                event("pad-a", 10, GilrsInputEventKind::Connected),
                event(
                    "pad-a",
                    12,
                    GilrsInputEventKind::ButtonChanged {
                        index: 3,
                        value: 1.0,
                    },
                ),
            ]),
        );

        let poll = backend.poll();

        assert_eq!(poll.device_events.len(), 1);
        let device = match &poll.device_events[0] {
            ohmygamepad_core::DeviceLifecycleEvent::Added(device) => device,
            other => panic!("unexpected device event: {other:?}"),
        };
        assert_eq!(device.device_id, "pad-a");
        assert_eq!(device.backend, Some(OhMyGamepadBackendKindDto::Sdl3));
        assert_eq!(
            device.connection,
            Some(OhMyGamepadConnectionKindDto::Bluetooth)
        );

        assert_eq!(poll.samples.len(), 1);
        assert_eq!(poll.samples[0].device_id, "pad-a");
        assert_eq!(poll.samples[0].observed_at_ms, 12);
        assert_eq!(poll.samples[0].buttons, vec![0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn poll_coalesces_multiple_input_changes_into_single_sample() {
        let mut backend = GilrsBackend::with_source(
            GilrsBackendConfig::default(),
            ScriptedGilrsSource::new(vec![
                event(
                    "pad-a",
                    10,
                    GilrsInputEventKind::ButtonChanged {
                        index: 1,
                        value: 1.0,
                    },
                ),
                event(
                    "pad-a",
                    11,
                    GilrsInputEventKind::AxisChanged {
                        index: 2,
                        value: 0.5,
                    },
                ),
                event(
                    "pad-a",
                    12,
                    GilrsInputEventKind::ButtonChanged {
                        index: 1,
                        value: 0.5,
                    },
                ),
            ]),
        );

        let poll = backend.poll();

        assert_eq!(poll.device_events.len(), 1);
        assert_eq!(poll.samples.len(), 1);
        assert_eq!(poll.samples[0].observed_at_ms, 12);
        assert_eq!(poll.samples[0].buttons, vec![0.0, 0.5]);
        assert_eq!(poll.samples[0].axes, vec![0.0, 0.0, 0.5]);
    }

    #[test]
    fn poll_emits_updated_when_descriptor_changes() {
        let mut backend = GilrsBackend::with_source(
            GilrsBackendConfig::default(),
            ScriptedGilrsSource::new(vec![
                event("pad-a", 10, GilrsInputEventKind::Connected),
                GilrsInputEvent {
                    device: GilrsDeviceDescriptor {
                        name: "Xbox Elite Wireless Controller".to_owned(),
                        ..descriptor("pad-a")
                    },
                    observed_at_ms: 20,
                    kind: GilrsInputEventKind::Connected,
                },
            ]),
        );

        let first_poll = backend.poll();
        assert_eq!(first_poll.device_events.len(), 2);
        assert!(first_poll.samples.is_empty());

        let updated = match &first_poll.device_events[1] {
            ohmygamepad_core::DeviceLifecycleEvent::Updated(device) => device,
            other => panic!("unexpected device event: {other:?}"),
        };
        assert_eq!(updated.name, "Xbox Elite Wireless Controller");
    }

    #[test]
    fn disconnect_clears_previous_button_state_before_reconnect() {
        let mut backend = GilrsBackend::with_source(
            GilrsBackendConfig::default(),
            ScriptedGilrsSource::new(vec![
                event(
                    "pad-a",
                    10,
                    GilrsInputEventKind::ButtonChanged {
                        index: 0,
                        value: 1.0,
                    },
                ),
                event("pad-a", 20, GilrsInputEventKind::Disconnected),
                event(
                    "pad-a",
                    30,
                    GilrsInputEventKind::ButtonChanged {
                        index: 1,
                        value: 1.0,
                    },
                ),
            ]),
        );

        let first_poll = backend.poll();
        assert_eq!(first_poll.samples.len(), 2);
        assert_eq!(first_poll.samples[0].buttons, vec![1.0]);
        assert_eq!(first_poll.samples[1].buttons, vec![0.0, 1.0]);
        assert!(matches!(
            first_poll.device_events[1],
            ohmygamepad_core::DeviceLifecycleEvent::Removed { .. }
        ));
    }

    #[test]
    fn max_events_per_poll_slices_source_workload() {
        let mut backend = GilrsBackend::with_source(
            GilrsBackendConfig {
                max_events_per_poll: 1,
                ..GilrsBackendConfig::default()
            },
            ScriptedGilrsSource::new(vec![
                event("pad-a", 10, GilrsInputEventKind::Connected),
                event(
                    "pad-a",
                    12,
                    GilrsInputEventKind::ButtonChanged {
                        index: 0,
                        value: 1.0,
                    },
                ),
            ]),
        );

        let first_poll = backend.poll();
        assert_eq!(first_poll.device_events.len(), 1);
        assert!(first_poll.samples.is_empty());

        let second_poll = backend.poll();
        assert!(second_poll.device_events.is_empty());
        assert_eq!(second_poll.samples.len(), 1);
        assert_eq!(second_poll.samples[0].buttons, vec![1.0]);
    }
}
