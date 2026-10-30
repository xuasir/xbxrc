use ohmygamepad_protocol::{LogicalPadBindingDto, OhMyGamepadSamplingConfigDto};

use crate::DeviceProfile;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DesktopBackendPreference {
    #[default]
    GilrsPreferred,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct InputCoreConfig {
    pub processing_mode: InputProcessingMode,
    pub backend_preference: DesktopBackendPreference,
    pub sampling: OhMyGamepadSamplingConfigDto,
    pub bindings: Vec<LogicalPadBindingDto>,
    pub device_profiles: Vec<DeviceProfile>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum InputProcessingMode {
    #[default]
    LogicalPads,
    /// 桌面公共控制器负责映射和归属，原生层提供每设备标准态。
    StandardDevices,
}
