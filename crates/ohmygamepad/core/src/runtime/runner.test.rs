use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use ohmygamepad_protocol::{
    LogicalPadSnapshotDto, OhMyGamepadCapabilityFlagsDto, OhMyGamepadDeviceClassificationDto,
    OhMyGamepadDeviceDto,
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
        product_version: None,
        firmware_version: None,
        serial_number: None,
        path: None,
        mapping: None,
        player_index: None,
        gamepad_type: None,
        power_state: None,
        battery_percent: None,
        touchpad_count: None,
        touchpad_finger_count: None,
        connected: true,
        last_seen_at_ms: 0,
        classification: OhMyGamepadDeviceClassificationDto::default(),
        sdl3_capabilities: OhMyGamepadCapabilityFlagsDto {
            supports_rumble: false,
            supports_trigger_rumble: false,
            reports_battery: false,
            supports_player_index: false,
            reports_mapping: false,
            supports_touchpad: false,
            supports_accel: false,
            supports_gyro: false,
            supports_led: false,
            reports_serial: false,
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
    let mut schedule = SamplingSchedule::new(&ohmygamepad_protocol::OhMyGamepadSamplingConfigDto {
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
    let mut schedule = SamplingSchedule::new(&ohmygamepad_protocol::OhMyGamepadSamplingConfigDto {
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
fn runtime_thread_emits_snapshot_and_accepts_input_policy_update() {
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
    assert_eq!(snapshot.slots.len(), 1);
    assert_eq!(snapshot.slots[0].state.buttons.south, 1.0);

    runtime
        .set_input_policy(ohmygamepad_protocol::OhMyGamepadInputPolicyDto::StreamOnly)
        .expect("input policy update should succeed");

    let updated_snapshot = runtime
        .get_runtime_snapshot()
        .expect("runtime snapshot should be available");
    assert_eq!(
        updated_snapshot.input_policy,
        ohmygamepad_protocol::OhMyGamepadInputPolicyDto::StreamOnly
    );
    assert!(wait_until(Duration::from_millis(80), || {
        !pads.lock().expect("lock pads").is_empty()
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
            slot: ohmygamepad_protocol::LogicalPadId::Pad0,
            mode: ohmygamepad_protocol::OhMyGamepadBindingModeDto::FixedDevice,
            device_ids: vec!["pad-b".to_owned()],
        })
        .expect("rebind should succeed");

    let snapshot = runtime
        .get_runtime_snapshot()
        .expect("runtime snapshot should be available");
    assert_eq!(snapshot.slot_bindings.len(), 1);
    assert_eq!(
        snapshot.slot_bindings[0].device_ids,
        vec!["pad-b".to_owned()]
    );

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
    assert_eq!(snapshot.slots.len(), 1);
    assert_eq!(snapshot.slots[0].state.buttons.south, 1.0);
    assert_eq!(snapshot.slots[0].state.buttons.north, 0.0);

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
        .set_input_policy(ohmygamepad_protocol::OhMyGamepadInputPolicyDto::StreamOnly)
        .expect("input policy update should succeed");
    let routed_snapshot = snapshot_rx
        .recv_timeout(Duration::from_millis(100))
        .expect("input policy snapshot should be pushed");
    assert_eq!(
        routed_snapshot.input_policy,
        ohmygamepad_protocol::OhMyGamepadInputPolicyDto::StreamOnly
    );

    runtime.shutdown().expect("runtime should shutdown cleanly");
}

#[test]
fn runtime_snapshot_subscription_respects_ui_push_rate() {
    let backend = ScriptedBackend::new(
        (0..8)
            .map(|_| BackendPollResult {
                device_events: vec![DeviceLifecycleEvent::Added(device("pad-a"))],
                samples: vec![sample("pad-a", 10, vec![1.0])],
            })
            .collect(),
    );
    let mut config = InputCoreConfig::default();
    config.sampling.backend_poll_rate_hz = 500;
    config.sampling.logical_pad_sample_rate_hz = 500;
    config.sampling.ui_push_rate_hz = 5;
    let runtime = spawn_input_runtime(
        config,
        backend,
        ThreadSafeUiSink::default(),
        ThreadSafeStreamSink,
    );
    let snapshot_rx = runtime.subscribe_runtime_snapshot();

    let started_at = Instant::now();
    let mut snapshots = Vec::new();
    while started_at.elapsed() < Duration::from_millis(180) {
        match snapshot_rx.recv_timeout(Duration::from_millis(25)) {
            Ok(snapshot) => snapshots.push(snapshot),
            Err(_) => {}
        }
    }

    runtime.shutdown().expect("runtime should shutdown cleanly");

    assert!(
        snapshots.len() <= 2,
        "ui push rate should suppress high-frequency snapshot broadcasts, got {}",
        snapshots.len()
    );
}
