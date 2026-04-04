    use std::{
        collections::VecDeque,
        sync::Mutex,
        thread,
        time::{Duration, Instant},
    };

    use ohmygamepad_protocol::{
        LogicalButtonsStateDto, LogicalPadId, LogicalPadStateDto, OhMyGamepadCapabilityFlagsDto,
        OhMyGamepadRumbleEffectDto, OhMyGamepadRumbleRejectionReasonDto,
        OhMyGamepadRumbleRequestDto, OhMyGamepadRumbleTargetDto, OhMyGamepadServiceCommandDto,
    };

    use super::{
        MultiControllerSamplingMode, MultiControllerSamplingStrategy, OhMyGamepadService,
        OhMyGamepadServiceConfig, ServiceRumbleBackend, SimulatedGamepadDescriptor,
        KEYBOARD_FALLBACK_DEVICE_ID,
    };
    use crate::{GilrsDeviceDescriptor, GilrsInputEvent, GilrsInputEventKind, GilrsSource};

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

    #[derive(Debug, Default)]
    struct RecordingRumbleBackend {
        play_calls: Mutex<Vec<(Vec<String>, OhMyGamepadRumbleEffectDto)>>,
        stop_calls: Mutex<Vec<Vec<String>>>,
    }

    impl ServiceRumbleBackend for RecordingRumbleBackend {
        fn play_rumble(
            &self,
            device_ids: &[String],
            effect: &OhMyGamepadRumbleEffectDto,
        ) -> Result<(), ohmygamepad_core::InputRuntimeError> {
            self.play_calls
                .lock()
                .expect("lock play calls")
                .push((device_ids.to_vec(), effect.clone()));
            Ok(())
        }

        fn stop_rumble(
            &self,
            device_ids: &[String],
        ) -> Result<(), ohmygamepad_core::InputRuntimeError> {
            self.stop_calls
                .lock()
                .expect("lock stop calls")
                .push(device_ids.to_vec());
            Ok(())
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

    fn descriptor(device_id: &str) -> GilrsDeviceDescriptor {
        GilrsDeviceDescriptor {
            device_id: device_id.to_owned(),
            name: "Xbox Wireless Controller".to_owned(),
            ..GilrsDeviceDescriptor::default()
        }
    }

    fn rumble_descriptor(device_id: &str) -> GilrsDeviceDescriptor {
        let mut descriptor = descriptor(device_id);
        descriptor.capabilities = OhMyGamepadCapabilityFlagsDto {
            basic_rumble: true,
            advanced_haptics: false,
            battery: false,
        };
        descriptor
    }

    fn physical_event(
        device_id: &str,
        observed_at_ms: u64,
        kind: GilrsInputEventKind,
    ) -> GilrsInputEvent {
        GilrsInputEvent {
            device: descriptor(device_id),
            observed_at_ms,
            kind,
        }
    }

    fn physical_rumble_event(
        device_id: &str,
        observed_at_ms: u64,
        kind: GilrsInputEventKind,
    ) -> GilrsInputEvent {
        GilrsInputEvent {
            device: rumble_descriptor(device_id),
            observed_at_ms,
            kind,
        }
    }

    fn state_with_button(
        value: f32,
        button: fn(&mut LogicalButtonsStateDto) -> &mut f32,
    ) -> LogicalPadStateDto {
        let mut state = LogicalPadStateDto::default();
        *button(&mut state.buttons) = value;
        state
    }

    fn test_service_config() -> OhMyGamepadServiceConfig {
        OhMyGamepadServiceConfig {
            desktop_keyboard: None,
            ..OhMyGamepadServiceConfig::default()
        }
    }

    fn spawn_service_with_rumble_backend(
        source: ScriptedGilrsSource,
        rumble_backend: RecordingRumbleBackend,
    ) -> OhMyGamepadService {
        OhMyGamepadService::spawn_with_source_and_rumble(
            test_service_config(),
            source,
            Some(Box::new(rumble_backend)),
        )
    }

    #[test]
    fn service_uses_merge_strategy_by_default() {
        let service = OhMyGamepadService::spawn_with_source(
            test_service_config(),
            ScriptedGilrsSource::default(),
        );

        service
            .connect_simulated_gamepad(SimulatedGamepadDescriptor {
                device_id: "sim-a".to_owned(),
                name: "Sim A".to_owned(),
                ..SimulatedGamepadDescriptor::default()
            })
            .expect("connect simulated device");
        service
            .connect_simulated_gamepad(SimulatedGamepadDescriptor {
                device_id: "sim-b".to_owned(),
                name: "Sim B".to_owned(),
                ..SimulatedGamepadDescriptor::default()
            })
            .expect("connect simulated device");
        service
            .submit_simulated_state(
                "sim-a",
                state_with_button(1.0, |buttons| &mut buttons.south),
            )
            .expect("submit simulated state");
        service
            .submit_simulated_state("sim-b", state_with_button(1.0, |buttons| &mut buttons.east))
            .expect("submit simulated state");

        assert!(wait_until(Duration::from_millis(80), || {
            service
                .snapshot()
                .map(|snapshot| {
                    snapshot
                        .pads
                        .first()
                        .map(|pad| pad.state.buttons.south >= 1.0 && pad.state.buttons.east >= 1.0)
                        .unwrap_or(false)
                })
                .unwrap_or(false)
        }));

        let snapshot = service.snapshot().expect("snapshot should be available");
        assert_eq!(snapshot.pads[0].pad_id, LogicalPadId::Pad0);
        assert_eq!(snapshot.pads[0].state.buttons.south, 1.0);
        assert_eq!(snapshot.pads[0].state.buttons.east, 1.0);

        service.shutdown().expect("service should shutdown cleanly");
    }

    #[test]
    fn service_can_prefer_primary_sampling_device() {
        let service = OhMyGamepadService::spawn_with_source(
            test_service_config(),
            ScriptedGilrsSource::default(),
        );

        service
            .connect_simulated_gamepad(SimulatedGamepadDescriptor {
                device_id: "sim-a".to_owned(),
                name: "Sim A".to_owned(),
                ..SimulatedGamepadDescriptor::default()
            })
            .expect("connect simulated device");
        service
            .connect_simulated_gamepad(SimulatedGamepadDescriptor {
                device_id: "sim-b".to_owned(),
                name: "Sim B".to_owned(),
                ..SimulatedGamepadDescriptor::default()
            })
            .expect("connect simulated device");
        service
            .submit_simulated_state(
                "sim-a",
                state_with_button(1.0, |buttons| &mut buttons.south),
            )
            .expect("submit simulated state");
        service
            .submit_simulated_state("sim-b", state_with_button(1.0, |buttons| &mut buttons.east))
            .expect("submit simulated state");

        service
            .set_sampling_strategy(MultiControllerSamplingStrategy {
                mode: MultiControllerSamplingMode::PrimaryPreferred,
                primary_device_id: Some("sim-b".to_owned()),
                paused_device_ids: Vec::new(),
                enable_keyboard_fallback: true,
            })
            .expect("strategy update should succeed");

        assert!(wait_until(Duration::from_millis(80), || {
            service
                .snapshot()
                .map(|snapshot| {
                    snapshot
                        .pads
                        .first()
                        .map(|pad| pad.device_ids == vec!["sim-b".to_owned()])
                        .unwrap_or(false)
                })
                .unwrap_or(false)
        }));

        let snapshot = service.snapshot().expect("snapshot should be available");
        assert_eq!(snapshot.pads[0].device_ids, vec!["sim-b".to_owned()]);
        assert_eq!(snapshot.pads[0].state.buttons.south, 0.0);
        assert_eq!(snapshot.pads[0].state.buttons.east, 1.0);

        service.shutdown().expect("service should shutdown cleanly");
    }

    #[test]
    fn keyboard_fallback_is_discoverable_without_physical_gamepad() {
        let service = OhMyGamepadService::spawn_with_source(
            test_service_config(),
            ScriptedGilrsSource::default(),
        );

        assert!(wait_until(Duration::from_millis(80), || {
            service
                .list_devices()
                .map(|devices| {
                    devices
                        .iter()
                        .any(|device| device.device_id == KEYBOARD_FALLBACK_DEVICE_ID)
                })
                .unwrap_or(false)
        }));

        let devices = service.list_devices().expect("devices should be available");
        assert!(devices
            .iter()
            .any(|device| device.device_id == KEYBOARD_FALLBACK_DEVICE_ID));

        service
            .submit_keyboard_state(state_with_button(1.0, |buttons| &mut buttons.south))
            .expect("keyboard state should be accepted");

        assert!(wait_until(Duration::from_millis(80), || {
            service
                .snapshot()
                .map(|snapshot| snapshot.pads[0].state.buttons.south >= 1.0)
                .unwrap_or(false)
        }));

        service.shutdown().expect("service should shutdown cleanly");
    }

    #[test]
    fn keyboard_fallback_is_suppressed_when_physical_device_exists() {
        let service = OhMyGamepadService::spawn_with_source(
            test_service_config(),
            ScriptedGilrsSource::new(vec![physical_event(
                "pad-a",
                10,
                GilrsInputEventKind::Connected,
            )]),
        );

        assert!(wait_until(Duration::from_millis(80), || {
            service
                .list_devices()
                .map(|devices| devices.iter().any(|device| device.device_id == "pad-a"))
                .unwrap_or(false)
        }));

        service
            .submit_keyboard_state(state_with_button(1.0, |buttons| &mut buttons.south))
            .expect("keyboard state should be accepted");

        let snapshot = service.snapshot().expect("snapshot should be available");
        assert_eq!(snapshot.pads[0].device_ids, vec!["pad-a".to_owned()]);
        assert_eq!(snapshot.pads[0].state.buttons.south, 0.0);

        service.shutdown().expect("service should shutdown cleanly");
    }

    #[test]
    fn suppressed_keyboard_submit_returns_without_sync_wait() {
        let service = OhMyGamepadService::spawn_with_source(
            test_service_config(),
            ScriptedGilrsSource::new(vec![physical_event(
                "pad-a",
                10,
                GilrsInputEventKind::Connected,
            )]),
        );

        assert!(wait_until(Duration::from_millis(80), || {
            service
                .list_devices()
                .map(|devices| devices.iter().any(|device| device.device_id == "pad-a"))
                .unwrap_or(false)
        }));

        let started_at = Instant::now();
        service
            .submit_keyboard_state(state_with_button(1.0, |buttons| &mut buttons.south))
            .expect("keyboard state should be accepted");

        assert!(started_at.elapsed() < Duration::from_millis(20));

        service.shutdown().expect("service should shutdown cleanly");
    }

    #[test]
    fn service_supports_device_query_and_pause_resume() {
        let service = OhMyGamepadService::spawn_with_source(
            test_service_config(),
            ScriptedGilrsSource::default(),
        );

        service
            .connect_simulated_gamepad(SimulatedGamepadDescriptor {
                device_id: "sim-a".to_owned(),
                name: "Sim A".to_owned(),
                ..SimulatedGamepadDescriptor::default()
            })
            .expect("connect simulated device");
        service
            .submit_simulated_state(
                "sim-a",
                state_with_button(1.0, |buttons| &mut buttons.south),
            )
            .expect("submit simulated state");

        let device = service
            .get_device("sim-a")
            .expect("device query should succeed")
            .expect("device should exist");
        assert_eq!(device.name, "Sim A");

        service
            .pause_sampling_device("sim-a")
            .expect("pause should succeed");
        let paused = service.snapshot().expect("snapshot should be available");
        assert!(paused.pads[0].device_ids.is_empty());
        assert_eq!(paused.pads[0].state.buttons.south, 0.0);

        service
            .resume_sampling_device("sim-a")
            .expect("resume should succeed");
        assert!(wait_until(Duration::from_millis(80), || {
            service
                .snapshot()
                .map(|snapshot| snapshot.pads[0].device_ids == vec!["sim-a".to_owned()])
                .unwrap_or(false)
        }));

        service.shutdown().expect("service should shutdown cleanly");
    }

    #[test]
    fn service_can_apply_command_facade() {
        let service = OhMyGamepadService::spawn_with_source(
            test_service_config(),
            ScriptedGilrsSource::default(),
        );

        service
            .apply_command(OhMyGamepadServiceCommandDto::ConnectSimulatedGamepad {
                descriptor: SimulatedGamepadDescriptor {
                    device_id: "sim-a".to_owned(),
                    name: "Sim A".to_owned(),
                    ..SimulatedGamepadDescriptor::default()
                },
            })
            .expect("connect command should succeed");
        service
            .apply_command(OhMyGamepadServiceCommandDto::SubmitSimulatedState {
                device_id: "sim-a".to_owned(),
                state: state_with_button(1.0, |buttons| &mut buttons.south),
            })
            .expect("submit command should succeed");

        assert!(wait_until(Duration::from_millis(80), || {
            service
                .snapshot()
                .map(|snapshot| snapshot.pads[0].state.buttons.south >= 1.0)
                .unwrap_or(false)
        }));

        service.shutdown().expect("service should shutdown cleanly");
    }

    #[test]
    fn service_config_enables_desktop_keyboard_listener_by_default() {
        assert!(OhMyGamepadServiceConfig::default()
            .desktop_keyboard
            .is_some());
    }

    #[test]
    fn play_rumble_reports_target_not_found_for_unknown_device() {
        let service = OhMyGamepadService::spawn_with_source(
            test_service_config(),
            ScriptedGilrsSource::default(),
        );

        let result = service
            .play_rumble(OhMyGamepadRumbleRequestDto {
                target: OhMyGamepadRumbleTargetDto::Device {
                    device_id: "missing".to_owned(),
                },
                effect: OhMyGamepadRumbleEffectDto::default(),
            })
            .expect("rumble request should return structured result");

        assert_eq!(result.accepted, false);
        assert_eq!(
            result.reason,
            Some(OhMyGamepadRumbleRejectionReasonDto::TargetNotFound)
        );
        assert!(result.resolved_device_ids.is_empty());

        service.shutdown().expect("service should shutdown cleanly");
    }

    #[test]
    fn play_rumble_dispatches_to_backend_for_supported_device() {
        let service = spawn_service_with_rumble_backend(
            ScriptedGilrsSource::new(vec![physical_rumble_event(
                "pad-a",
                10,
                GilrsInputEventKind::Connected,
            )]),
            RecordingRumbleBackend::default(),
        );

        assert!(wait_until(Duration::from_millis(80), || {
            service
                .list_devices()
                .map(|devices| devices.iter().any(|device| device.device_id == "pad-a"))
                .unwrap_or(false)
        }));

        let result = service
            .play_rumble(OhMyGamepadRumbleRequestDto {
                target: OhMyGamepadRumbleTargetDto::Device {
                    device_id: "pad-a".to_owned(),
                },
                effect: OhMyGamepadRumbleEffectDto {
                    strong_magnitude: 0.8,
                    ..OhMyGamepadRumbleEffectDto::default()
                },
            })
            .expect("rumble request should return structured result");

        assert!(result.accepted);
        assert_eq!(result.reason, None);
        assert_eq!(result.resolved_device_ids, vec!["pad-a".to_owned()]);

        service.shutdown().expect("service should shutdown cleanly");
    }

    #[test]
    fn play_rumble_dispatches_to_backend_for_gilrs_device_without_capability_flag() {
        let service = spawn_service_with_rumble_backend(
            ScriptedGilrsSource::new(vec![physical_event(
                "pad-a",
                10,
                GilrsInputEventKind::Connected,
            )]),
            RecordingRumbleBackend::default(),
        );

        assert!(wait_until(Duration::from_millis(80), || {
            service
                .list_devices()
                .map(|devices| devices.iter().any(|device| device.device_id == "pad-a"))
                .unwrap_or(false)
        }));

        let result = service
            .play_rumble(OhMyGamepadRumbleRequestDto {
                target: OhMyGamepadRumbleTargetDto::Device {
                    device_id: "pad-a".to_owned(),
                },
                effect: OhMyGamepadRumbleEffectDto {
                    weak_magnitude: 0.6,
                    ..OhMyGamepadRumbleEffectDto::default()
                },
            })
            .expect("rumble request should still dispatch for gilrs device");

        assert!(result.accepted);
        assert_eq!(result.reason, None);
        assert_eq!(result.resolved_device_ids, vec!["pad-a".to_owned()]);

        service.shutdown().expect("service should shutdown cleanly");
    }

    #[test]
    fn stop_rumble_dispatches_to_backend_for_supported_device() {
        let service = spawn_service_with_rumble_backend(
            ScriptedGilrsSource::new(vec![physical_rumble_event(
                "pad-a",
                10,
                GilrsInputEventKind::Connected,
            )]),
            RecordingRumbleBackend::default(),
        );

        assert!(wait_until(Duration::from_millis(80), || {
            service
                .list_devices()
                .map(|devices| devices.iter().any(|device| device.device_id == "pad-a"))
                .unwrap_or(false)
        }));

        let result = service
            .stop_rumble(OhMyGamepadRumbleTargetDto::Device {
                device_id: "pad-a".to_owned(),
            })
            .expect("stop rumble should return structured result");

        assert!(result.accepted);
        assert_eq!(result.reason, None);
        assert_eq!(result.resolved_device_ids, vec!["pad-a".to_owned()]);

        service.shutdown().expect("service should shutdown cleanly");
    }
