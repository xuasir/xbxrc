use ohmygamepad_protocol::{
    OhMyGamepadBackendKindDto, OhMyGamepadCapabilityFlagsDto, OhMyGamepadConnectionKindDto,
    OhMyGamepadDeviceDto,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GilrsDeviceDescriptor {
    pub device_id: String,
    pub name: String,
    pub connection: Option<OhMyGamepadConnectionKindDto>,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub capabilities: OhMyGamepadCapabilityFlagsDto,
}

// 为 SDL3 迁移预留中立命名，避免上层继续绑定具体驱动名。
pub type InputDeviceDescriptor = GilrsDeviceDescriptor;

impl GilrsDeviceDescriptor {
    pub fn to_connected_device(&self, observed_at_ms: u64) -> OhMyGamepadDeviceDto {
        OhMyGamepadDeviceDto {
            device_id: self.device_id.clone(),
            name: self.name.clone(),
            backend: Some(OhMyGamepadBackendKindDto::Sdl3),
            connection: self.connection,
            vendor_id: self.vendor_id,
            product_id: self.product_id,
            connected: true,
            last_seen_at_ms: observed_at_ms,
            capabilities: self.capabilities,
            effective_capabilities: self.capabilities,
            is_default_target: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum GilrsInputEventKind {
    Connected,
    Disconnected,
    ButtonChanged { index: usize, value: f32 },
    AxisChanged { index: usize, value: f32 },
    Dropped,
}

pub type InputEventKind = GilrsInputEventKind;

#[derive(Clone, Debug, PartialEq)]
pub struct GilrsInputEvent {
    pub device: GilrsDeviceDescriptor,
    pub observed_at_ms: u64,
    pub kind: GilrsInputEventKind,
}

pub type InputEvent = GilrsInputEvent;
