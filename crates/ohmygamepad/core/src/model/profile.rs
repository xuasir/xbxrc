use ohmygamepad_protocol::{OhMyGamepadBackendKindDto, OhMyGamepadDeviceDto};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilterConfig {
    pub stick_deadzone: f32,
    pub stick_epsilon: f32,
    pub trigger_deadzone: f32,
    pub trigger_epsilon: f32,
    pub button_epsilon: f32,
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            stick_deadzone: 0.10,
            stick_epsilon: 0.002,
            trigger_deadzone: 0.03,
            trigger_epsilon: 0.01,
            button_epsilon: 0.0001,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ButtonMapping {
    pub south: usize,
    pub east: usize,
    pub west: usize,
    pub north: usize,
    pub l1: usize,
    pub r1: usize,
    pub l2: usize,
    pub r2: usize,
    pub view: usize,
    pub menu: usize,
    pub l3: usize,
    pub r3: usize,
    pub dpad_up: usize,
    pub dpad_down: usize,
    pub dpad_left: usize,
    pub dpad_right: usize,
    pub home: usize,
}

impl Default for ButtonMapping {
    fn default() -> Self {
        Self {
            south: 0,
            east: 1,
            west: 2,
            north: 3,
            l1: 4,
            r1: 5,
            l2: 6,
            r2: 7,
            view: 8,
            menu: 9,
            l3: 10,
            r3: 11,
            dpad_up: 12,
            dpad_down: 13,
            dpad_left: 14,
            dpad_right: 15,
            home: 16,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisMapping {
    pub left_stick_x: usize,
    pub left_stick_y: usize,
    pub right_stick_x: usize,
    pub right_stick_y: usize,
    pub left_trigger_button: usize,
    pub right_trigger_button: usize,
    pub left_trigger_axis: Option<usize>,
    pub right_trigger_axis: Option<usize>,
}

impl Default for AxisMapping {
    fn default() -> Self {
        Self {
            left_stick_x: 0,
            left_stick_y: 1,
            right_stick_x: 2,
            right_stick_y: 3,
            left_trigger_button: 6,
            right_trigger_button: 7,
            left_trigger_axis: Some(4),
            right_trigger_axis: Some(5),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeviceProfileMatcher {
    pub device_id: Option<String>,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub backend: Option<OhMyGamepadBackendKindDto>,
    pub name_contains: Option<String>,
}

impl DeviceProfileMatcher {
    pub fn matches(&self, device: &OhMyGamepadDeviceDto) -> bool {
        if let Some(device_id) = &self.device_id {
            if &device.device_id != device_id {
                return false;
            }
        }
        if let Some(vendor_id) = self.vendor_id {
            if device.vendor_id != Some(vendor_id) {
                return false;
            }
        }
        if let Some(product_id) = self.product_id {
            if device.product_id != Some(product_id) {
                return false;
            }
        }
        if let Some(backend) = self.backend {
            if device.backend != Some(backend) {
                return false;
            }
        }
        if let Some(name_contains) = &self.name_contains {
            let pattern = name_contains.trim().to_ascii_lowercase();
            if pattern.is_empty() || !device.name.to_ascii_lowercase().contains(&pattern) {
                return false;
            }
        }
        true
    }

    pub fn match_score(&self) -> usize {
        usize::from(self.device_id.is_some()) * 100
            + usize::from(self.vendor_id.is_some()) * 10
            + usize::from(self.product_id.is_some()) * 10
            + usize::from(self.backend.is_some()) * 5
            + usize::from(self.name_contains.is_some()) * 2
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeviceProfile {
    pub matcher: DeviceProfileMatcher,
    pub buttons: ButtonMapping,
    pub axes: AxisMapping,
    pub filter: FilterConfig,
}

#[cfg(test)]
mod tests {
    use ohmygamepad_protocol::{
        OhMyGamepadBackendKindDto, OhMyGamepadCapabilityFlagsDto,
        OhMyGamepadDeviceClassificationDto, OhMyGamepadDeviceDto,
    };

    use super::DeviceProfileMatcher;

    fn device() -> OhMyGamepadDeviceDto {
        OhMyGamepadDeviceDto {
            device_id: "device-a".to_owned(),
            name: "Xbox Wireless Controller".to_owned(),
            backend: Some(OhMyGamepadBackendKindDto::Sdl3),
            connection: None,
            vendor_id: Some(0x045e),
            product_id: Some(0x0b13),
            product_version: None,
            firmware_version: None,
            serial_number: None,
            path: None,
            mapping: None,
            player_index: None,
            gamepad_type: None,
            power_state: None,
            battery_percent: None,
            touchpad_count: None,
            touchpad_finger_count: None,
            connected: true,
            last_seen_at_ms: 0,
            classification: OhMyGamepadDeviceClassificationDto::default(),
            sdl3_capabilities: OhMyGamepadCapabilityFlagsDto::default(),
        }
    }

    #[test]
    fn matcher_accepts_vendor_product_and_name_pattern() {
        let matcher = DeviceProfileMatcher {
            vendor_id: Some(0x045e),
            product_id: Some(0x0b13),
            name_contains: Some("wireless".to_owned()),
            ..DeviceProfileMatcher::default()
        };

        assert!(matcher.matches(&device()));
    }

    #[test]
    fn matcher_rejects_device_when_any_constraint_fails() {
        let matcher = DeviceProfileMatcher {
            backend: Some(OhMyGamepadBackendKindDto::Mock),
            ..DeviceProfileMatcher::default()
        };

        assert!(!matcher.matches(&device()));
    }
}
