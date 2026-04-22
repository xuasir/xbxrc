use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OhMyGamepadBackendKindDto {
    Gilrs,
    Sdl3,
    Mock,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OhMyGamepadConnectionKindDto {
    Usb,
    Bluetooth,
    WirelessDongle,
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OhMyGamepadHapticsProviderKindDto {
    #[default]
    Sdl3Gamepad,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OhMyGamepadCapabilityFlagsDto {
    pub basic_rumble: bool,
    pub advanced_haptics: bool,
    pub battery: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OhMyGamepadDeviceDto {
    pub device_id: String,
    pub name: String,
    pub backend: Option<OhMyGamepadBackendKindDto>,
    pub connection: Option<OhMyGamepadConnectionKindDto>,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub connected: bool,
    pub last_seen_at_ms: u64,
    pub capabilities: OhMyGamepadCapabilityFlagsDto,
    pub effective_capabilities: OhMyGamepadCapabilityFlagsDto,
    pub is_default_target: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OhMyGamepadRuntimeHapticsDto {
    pub provider: OhMyGamepadHapticsProviderKindDto,
    pub supports_auto_target: bool,
    pub supports_basic_rumble: bool,
    pub supports_advanced_haptics: bool,
    pub default_device_id: Option<String>,
}
