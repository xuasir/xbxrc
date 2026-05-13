use ohmygamepad_protocol::OhMyGamepadDeviceDto;

#[derive(Clone, Debug, PartialEq)]
pub enum DeviceLifecycleEvent {
    Added(OhMyGamepadDeviceDto),
    Updated(OhMyGamepadDeviceDto),
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
    /// SDL 等后端在「硬件读数未变化」时也可能不产出 `samples`；用该时间戳表示本轮确实轮询过设备，
    /// 以便 `last_backend_sample_activity_at_ms` 仍能反映采样线程存活（首开 SDL 签名冻结自愈依赖）。
    pub activity_observed_at_ms: Option<u64>,
}

pub trait InputBackend {
    fn poll(&mut self) -> BackendPollResult;
}
