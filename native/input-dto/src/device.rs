#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GamepadBackendKindDto {
    Gilrs,
    Mock,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GamepadConnectionKindDto {
    Usb,
    Bluetooth,
    WirelessDongle,
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GamepadCapabilityFlagsDto {
    pub basic_rumble: bool,
    pub advanced_haptics: bool,
    pub battery: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GamepadDeviceDto {
    pub device_id: String,
    pub name: String,
    pub backend: Option<GamepadBackendKindDto>,
    pub connection: Option<GamepadConnectionKindDto>,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub connected: bool,
    pub last_seen_at_ms: u64,
    pub capabilities: GamepadCapabilityFlagsDto,
}
