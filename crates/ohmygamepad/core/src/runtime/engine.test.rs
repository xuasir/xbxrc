use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use ohmygamepad_protocol::{
    LogicalButtonsStateDto, OhMyGamepadBackendKindDto, OhMyGamepadBindingModeDto,
    OhMyGamepadCapabilityFlagsDto, OhMyGamepadDeviceDto,
};

use super::InputCore;
use crate::{
    BackendPollResult, ButtonMapping, DeviceLifecycleEvent, DeviceProfile, DeviceProfileMatcher,
    FilterConfig, InputBackend, InputCoreConfig, RawDeviceSample, StreamSink, UiSink,
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
        sdl3_capabilities: OhMyGamepadCapabilityFlagsDto {
            supports_rumble: false,
            supports_trigger_rumble: false,
            reports_battery: false,
            supports_player_index: false,
            reports_mapping: false,
            supports_touchpad: false,
            supports_led: false,
            reports_serial: false,
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
    assert_eq!(latest.slot, ohmygamepad_protocol::LogicalPadId::Pad0);
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
        slot: ohmygamepad_protocol::LogicalPadId::Pad0,
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
        slot: ohmygamepad_protocol::LogicalPadId::Pad0,
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
            slot: ohmygamepad_protocol::LogicalPadId::Pad0,
            mode: OhMyGamepadBindingModeDto::Split,
            device_ids: Vec::new(),
        },
        ohmygamepad_protocol::LogicalPadBindingDto {
            slot: ohmygamepad_protocol::LogicalPadId::Pad1,
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
        slot: ohmygamepad_protocol::LogicalPadId::Pad0,
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
fn input_policy_change_emits_new_snapshot() {
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
    core.replace_input_policy(ohmygamepad_protocol::OhMyGamepadInputPolicyDto::StreamOnly);
    core.sample_once();

    let pads = ui_pads.borrow();
    assert_eq!(pads.len(), 1);
    assert_eq!(pads[0].slot, ohmygamepad_protocol::LogicalPadId::Pad0);
    drop(pads);
    assert_eq!(
        core.runtime_snapshot().input_policy,
        ohmygamepad_protocol::OhMyGamepadInputPolicyDto::StreamOnly
    );
}

#[test]
fn unchanged_snapshot_preserves_sample_seq_in_on_change_mode() {
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
        SharedUiSink::default(),
        SharedStreamSink::default(),
    );

    core.sync_clock_ms(10);
    core.tick();
    let first_seq = core.runtime_snapshot().slots[0].sample_seq;

    core.sync_clock_ms(20);
    core.tick();
    let second_seq = core.runtime_snapshot().slots[0].sample_seq;

    assert_eq!(first_seq, second_seq);
}

#[test]
fn fixed_rate_stream_mode_advances_sample_seq_without_payload_change() {
    let backend = ScriptedBackend::new(vec![
        BackendPollResult {
            device_events: vec![DeviceLifecycleEvent::Added(device("pad-a"))],
            samples: vec![sample("pad-a", 10, vec![1.0], vec![0.0, 0.0, 0.0, 0.0])],
        },
        BackendPollResult {
            device_events: vec![],
            samples: vec![sample("pad-a", 10, vec![1.0], vec![0.0, 0.0, 0.0, 0.0])],
        },
    ]);
    let mut config = InputCoreConfig::default();
    config.sampling.stream_push_mode =
        ohmygamepad_protocol::OhMyGamepadStreamPushModeDto::FixedRate;
    config.sampling.stream_push_rate_hz = Some(10);
    let mut core = InputCore::new(config, backend, SharedUiSink::default(), SharedStreamSink::default());

    core.sync_clock_ms(10);
    core.tick();
    let first_snapshot = core.runtime_snapshot().slots[0].clone();

    core.sync_clock_ms(50);
    core.tick();
    let second_snapshot = core.runtime_snapshot().slots[0].clone();
    assert_eq!(first_snapshot.sample_seq, second_snapshot.sample_seq);

    core.sync_clock_ms(120);
    core.sample_once();
    let third_snapshot = core.runtime_snapshot().slots[0].clone();
    assert!(third_snapshot.sample_seq > second_snapshot.sample_seq);
    assert_eq!(third_snapshot.state, second_snapshot.state);
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
    first_device.backend = Some(OhMyGamepadBackendKindDto::Sdl3);
    first_device.vendor_id = Some(0x045e);
    first_device.product_id = Some(0x0b13);

    let mut second_device = device("pad-a-reconnected");
    second_device.name = "Xbox Wireless Controller".to_owned();
    second_device.backend = Some(OhMyGamepadBackendKindDto::Sdl3);
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
            backend: Some(OhMyGamepadBackendKindDto::Sdl3),
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
    matched_device.backend = Some(OhMyGamepadBackendKindDto::Sdl3);
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
                backend: Some(OhMyGamepadBackendKindDto::Sdl3),
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
