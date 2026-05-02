use ohmygamepad_protocol::{
    OhMyGamepadBackendKindDto, OhMyGamepadCapabilityFlagsDto, OhMyGamepadConnectionKindDto,
    OhMyGamepadDeviceClassificationDto, OhMyGamepadDeviceDto, OhMyGamepadDeviceTypeDto,
    OhMyGamepadPowerStateDto,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Sdl3DeviceDescriptor {
    pub device_id: String,
    pub name: String,
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
    pub classification: OhMyGamepadDeviceClassificationDto,
    pub capabilities: OhMyGamepadCapabilityFlagsDto,
}

impl Sdl3DeviceDescriptor {
    pub fn to_connected_device(&self, observed_at_ms: u64) -> OhMyGamepadDeviceDto {
        OhMyGamepadDeviceDto {
            device_id: self.device_id.clone(),
            name: self.name.clone(),
            backend: Some(OhMyGamepadBackendKindDto::Sdl3),
            connection: self.connection,
            vendor_id: self.vendor_id,
            product_id: self.product_id,
            product_version: self.product_version,
            firmware_version: self.firmware_version,
            serial_number: self.serial_number.clone(),
            path: self.path.clone(),
            mapping: self.mapping.clone(),
            player_index: self.player_index,
            gamepad_type: self.gamepad_type,
            power_state: self.power_state,
            battery_percent: self.battery_percent,
            touchpad_count: self.touchpad_count,
            touchpad_finger_count: self.touchpad_finger_count,
            connected: true,
            last_seen_at_ms: observed_at_ms,
            classification: self.classification.clone(),
            sdl3_capabilities: self.capabilities,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Sdl3InputEventKind {
    Connected,
    Disconnected,
    Snapshot { buttons: Vec<f32>, axes: Vec<f32> },
    ButtonChanged { index: usize, value: f32 },
    AxisChanged { index: usize, value: f32 },
    Dropped,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sdl3InputEvent {
    pub device: Sdl3DeviceDescriptor,
    pub observed_at_ms: u64,
    pub kind: Sdl3InputEventKind,
}
