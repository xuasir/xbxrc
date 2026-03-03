use input_dto::{
    GamepadDeviceDto, GamepadRouteTargetDto, GamepadRuntimeSnapshotDto, GamepadSamplingConfigDto,
    LogicalPadBindingDto, LogicalPadId, LogicalPadSnapshotDto,
};

use crate::{
    BackendPollResult, DeviceLifecycleEvent, InputBackend, InputCoreConfig, StreamSink, UiSink,
};

pub struct InputCore<TBackend, TUiSink, TStreamSink> {
    config: InputCoreConfig,
    backend: TBackend,
    ui_sink: TUiSink,
    stream_sink: TStreamSink,
    devices: Vec<GamepadDeviceDto>,
    pads: Vec<LogicalPadSnapshotDto>,
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
        }
    }

    pub fn config(&self) -> &InputCoreConfig {
        &self.config
    }

    pub fn runtime_snapshot(&self) -> GamepadRuntimeSnapshotDto {
        GamepadRuntimeSnapshotDto {
            devices: self.devices.clone(),
            bindings: self.config.bindings.clone(),
            route_target: self.config.route_target.clone(),
            sampling: self.config.sampling.clone(),
            pads: self.pads.clone(),
        }
    }

    pub fn replace_sampling_config(&mut self, sampling: GamepadSamplingConfigDto) {
        self.config.sampling = sampling;
    }

    pub fn replace_route_target(&mut self, route_target: GamepadRouteTargetDto) {
        self.config.route_target = route_target;
    }

    pub fn replace_bindings(&mut self, bindings: Vec<LogicalPadBindingDto>) {
        self.config.bindings = bindings;
    }

    pub fn tick(&mut self) {
        let poll = self.backend.poll();
        self.apply_backend_poll(poll);
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

    fn apply_backend_poll(&mut self, poll: BackendPollResult) {
        let BackendPollResult {
            device_events,
            samples,
        } = poll;
        let mut devices_changed = false;

        for event in device_events {
            devices_changed |= self.apply_device_event(event);
        }

        // 当前阶段先把设备表与桥接 DTO 稳定下来，采样样本后续接入映射/过滤链路。
        let _ = samples;

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
                previous_len != self.devices.len()
            }
        }
    }

    fn upsert_device(&mut self, device: GamepadDeviceDto) -> bool {
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
}
