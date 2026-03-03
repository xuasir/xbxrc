use input_dto::GamepadDeviceDto;

#[derive(Clone, Debug, PartialEq)]
pub enum DeviceLifecycleEvent {
    Added(GamepadDeviceDto),
    Updated(GamepadDeviceDto),
    Removed {
        device_id: String,
        observed_at_ms: u64,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RawDeviceSample {
    pub device_id: String,
    pub observed_at_ms: u64,
    pub buttons: Vec<f32>,
    pub axes: Vec<f32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BackendPollResult {
    pub device_events: Vec<DeviceLifecycleEvent>,
    pub samples: Vec<RawDeviceSample>,
}

pub trait InputBackend {
    fn poll(&mut self) -> BackendPollResult;
}
