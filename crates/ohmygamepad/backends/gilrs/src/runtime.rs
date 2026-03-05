use ohmygamepad_core::{
    spawn_input_runtime, InputCoreConfig, InputRuntimeHandle, StreamSink, UiSink,
};

use crate::{GilrsBackend, GilrsBackendConfig, GilrsSource, GilrsSourceInitError};

pub fn spawn_gilrs_input_runtime<TUiSink, TStreamSink>(
    core_config: InputCoreConfig,
    backend_config: GilrsBackendConfig,
    ui_sink: TUiSink,
    stream_sink: TStreamSink,
) -> Result<InputRuntimeHandle, GilrsSourceInitError>
where
    TUiSink: UiSink + Send + 'static,
    TStreamSink: StreamSink + Send + 'static,
{
    let backend = GilrsBackend::new(backend_config)?;
    Ok(spawn_input_runtime(
        core_config,
        backend,
        ui_sink,
        stream_sink,
    ))
}

pub fn spawn_gilrs_input_runtime_with_source<TSource, TUiSink, TStreamSink>(
    core_config: InputCoreConfig,
    backend_config: GilrsBackendConfig,
    source: TSource,
    ui_sink: TUiSink,
    stream_sink: TStreamSink,
) -> InputRuntimeHandle
where
    TSource: GilrsSource + Send + 'static,
    TUiSink: UiSink + Send + 'static,
    TStreamSink: StreamSink + Send + 'static,
{
    // 测试与离线仿真都复用同一条 runtime 线程链路，避免只测 backend 聚合器而漏掉 ohmygamepad-core 集成问题。
    let backend = GilrsBackend::with_source(backend_config, source);
    spawn_input_runtime(core_config, backend, ui_sink, stream_sink)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
        thread,
        time::{Duration, Instant},
    };

    use ohmygamepad_core::{InputCoreConfig, StreamSink, UiSink};
    use ohmygamepad_protocol::{LogicalPadSnapshotDto, OhMyGamepadDeviceDto};

    use super::spawn_gilrs_input_runtime_with_source;
    use crate::{
        GilrsBackendConfig, GilrsDeviceDescriptor, GilrsInputEvent, GilrsInputEventKind,
        GilrsSource,
    };

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

    #[derive(Default, Clone)]
    struct ThreadSafeUiSink {
        devices: Arc<Mutex<Vec<Vec<OhMyGamepadDeviceDto>>>>,
        pads: Arc<Mutex<Vec<LogicalPadSnapshotDto>>>,
    }

    impl UiSink for ThreadSafeUiSink {
        fn emit_devices_changed(&mut self, devices: &[OhMyGamepadDeviceDto]) {
            self.devices
                .lock()
                .expect("lock devices")
                .push(devices.to_vec());
        }

        fn emit_pad_snapshot(&mut self, snapshot: &LogicalPadSnapshotDto) {
            self.pads.lock().expect("lock pads").push(snapshot.clone());
        }
    }

    #[derive(Default, Clone)]
    struct ThreadSafeStreamSink;

    impl StreamSink for ThreadSafeStreamSink {
        fn emit_pad_snapshot(&mut self, _snapshot: &LogicalPadSnapshotDto) {}
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

    fn event(device_id: &str, observed_at_ms: u64, kind: GilrsInputEventKind) -> GilrsInputEvent {
        GilrsInputEvent {
            device: descriptor(device_id),
            observed_at_ms,
            kind,
        }
    }

    #[test]
    fn scripted_gilrs_runtime_produces_logical_pad_snapshot() {
        let ui_sink = ThreadSafeUiSink::default();
        let pads = ui_sink.pads.clone();

        let runtime = spawn_gilrs_input_runtime_with_source(
            InputCoreConfig::default(),
            GilrsBackendConfig::default(),
            ScriptedGilrsSource::new(vec![
                event("pad-a", 10, GilrsInputEventKind::Connected),
                event(
                    "pad-a",
                    11,
                    GilrsInputEventKind::ButtonChanged {
                        index: 0,
                        value: 1.0,
                    },
                ),
                event(
                    "pad-a",
                    12,
                    GilrsInputEventKind::AxisChanged {
                        index: 0,
                        value: 0.75,
                    },
                ),
            ]),
            ui_sink,
            ThreadSafeStreamSink,
        );

        assert!(wait_until(Duration::from_millis(80), || {
            !pads.lock().expect("lock pads").is_empty()
        }));

        let snapshot = runtime
            .get_runtime_snapshot()
            .expect("runtime snapshot should be available");
        assert_eq!(snapshot.devices.len(), 1);
        assert_eq!(snapshot.pads.len(), 1);
        assert_eq!(snapshot.pads[0].state.buttons.south, 1.0);
        assert!(snapshot.pads[0].state.left_stick.x > 0.7);

        runtime.shutdown().expect("runtime should shutdown cleanly");
    }

    #[test]
    fn scripted_gilrs_runtime_maps_dpad_axis_into_button_state() {
        let runtime = spawn_gilrs_input_runtime_with_source(
            InputCoreConfig::default(),
            GilrsBackendConfig::default(),
            ScriptedGilrsSource::new(vec![
                event("pad-a", 10, GilrsInputEventKind::Connected),
                event(
                    "pad-a",
                    12,
                    GilrsInputEventKind::ButtonChanged {
                        index: 14,
                        value: 0.0,
                    },
                ),
                event(
                    "pad-a",
                    12,
                    GilrsInputEventKind::ButtonChanged {
                        index: 15,
                        value: 1.0,
                    },
                ),
            ]),
            ThreadSafeUiSink::default(),
            ThreadSafeStreamSink,
        );

        assert!(wait_until(Duration::from_millis(80), || {
            runtime
                .get_runtime_snapshot()
                .map(|snapshot| {
                    snapshot
                        .pads
                        .first()
                        .map(|pad| pad.state.buttons.dpad_right >= 1.0)
                        .unwrap_or(false)
                })
                .unwrap_or(false)
        }));

        let snapshot = runtime
            .get_runtime_snapshot()
            .expect("runtime snapshot should be available");
        assert_eq!(snapshot.pads[0].state.buttons.dpad_left, 0.0);
        assert_eq!(snapshot.pads[0].state.buttons.dpad_right, 1.0);

        runtime.shutdown().expect("runtime should shutdown cleanly");
    }
}
