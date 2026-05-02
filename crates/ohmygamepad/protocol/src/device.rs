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
#[serde(rename_all = "kebab-case")]
pub enum OhMyGamepadPowerStateDto {
    #[default]
    Unknown,
    Wired,
    OnBattery,
    Charging,
    Charged,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OhMyGamepadDeviceTypeDto {
    #[default]
    Unknown,
    Standard,
    Xbox360,
    XboxOne,
    Ps3,
    Ps4,
    Ps5,
    NintendoSwitchPro,
    NintendoSwitchJoyconLeft,
    NintendoSwitchJoyconRight,
    NintendoSwitchJoyconPair,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OhMyGamepadCapabilityFlagsDto {
    pub supports_rumble: bool,
    pub supports_trigger_rumble: bool,
    pub reports_battery: bool,
    pub supports_player_index: bool,
    pub reports_mapping: bool,
    pub supports_touchpad: bool,
    pub supports_accel: bool,
    pub supports_gyro: bool,
    pub supports_led: bool,
    pub reports_serial: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OhMyGamepadDeviceDto {
    pub device_id: String,
    pub name: String,
    pub backend: Option<OhMyGamepadBackendKindDto>,
    pub connection: Option<OhMyGamepadConnectionKindDto>,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub product_version: Option<u16>,
    pub firmware_version: Option<u16>,
    pub serial_number: Option<String>,
    pub path: Option<String>,
    pub mapping: Option<String>,
    pub player_index: Option<u16>,
    pub gamepad_type: Option<OhMyGamepadDeviceTypeDto>,
    pub power_state: Option<OhMyGamepadPowerStateDto>,
    pub battery_percent: Option<u8>,
    pub touchpad_count: Option<u16>,
    pub touchpad_finger_count: Option<u16>,
    pub connected: bool,
    pub last_seen_at_ms: u64,
    pub sdl3_capabilities: OhMyGamepadCapabilityFlagsDto,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OhMyGamepadRuntimeHapticsDto {
    pub provider: OhMyGamepadHapticsProviderKindDto,
    pub supports_basic_rumble: bool,
    pub supports_trigger_rumble: bool,
    pub default_device_id: Option<String>,
}
