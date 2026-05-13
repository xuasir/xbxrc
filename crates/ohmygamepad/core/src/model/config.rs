use ohmygamepad_protocol::{LogicalPadBindingDto, OhMyGamepadSamplingConfigDto};

use crate::DeviceProfile;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DesktopBackendPreference {
    #[default]
    GilrsPreferred,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct InputCoreConfig {
    pub backend_preference: DesktopBackendPreference,
    pub sampling: OhMyGamepadSamplingConfigDto,
    pub bindings: Vec<LogicalPadBindingDto>,
    pub device_profiles: Vec<DeviceProfile>,
}
