use std::collections::BTreeSet;

use ohmygamepad_protocol::LogicalStickDto;
pub use ohmygamepad_protocol::{
    LogicalPadStateDto, OhMyGamepadKeyboardBindingDto as OhMyGamepadKeyboardBinding,
    OhMyGamepadKeyboardControlDto as OhMyGamepadKeyboardControl,
    OhMyGamepadKeyboardKeyDto as OhMyGamepadKeyboardKey,
    OhMyGamepadKeyboardMappingDto as OhMyGamepadKeyboardMapping,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OhMyGamepadKeyboardEvent {
    pub key: OhMyGamepadKeyboardKey,
    pub pressed: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OhMyGamepadKeyboardMapper {
    mapping: OhMyGamepadKeyboardMapping,
    pressed_keys: BTreeSet<OhMyGamepadKeyboardKey>,
}

impl OhMyGamepadKeyboardMapper {
    pub fn new(mapping: OhMyGamepadKeyboardMapping) -> Self {
        Self {
            mapping,
            pressed_keys: BTreeSet::new(),
        }
    }

    pub fn with_default_mapping() -> Self {
        Self::new(OhMyGamepadKeyboardMapping::default())
    }

    pub fn mapping(&self) -> &OhMyGamepadKeyboardMapping {
        &self.mapping
    }

    pub fn replace_mapping(&mut self, mapping: OhMyGamepadKeyboardMapping) -> LogicalPadStateDto {
        self.mapping = mapping;
        self.state()
    }

    pub fn state(&self) -> LogicalPadStateDto {
        let mut state = LogicalPadStateDto::default();

        for binding in &self.mapping.bindings {
            if !self.pressed_keys.contains(&binding.key) {
                continue;
            }

            match binding.control {
                OhMyGamepadKeyboardControl::LeftStickUp => state.left_stick.y += 1.0,
                OhMyGamepadKeyboardControl::LeftStickDown => state.left_stick.y -= 1.0,
                OhMyGamepadKeyboardControl::LeftStickLeft => state.left_stick.x -= 1.0,
                OhMyGamepadKeyboardControl::LeftStickRight => state.left_stick.x += 1.0,
                OhMyGamepadKeyboardControl::RightStickUp => state.right_stick.y -= 1.0,
                OhMyGamepadKeyboardControl::RightStickDown => state.right_stick.y += 1.0,
                OhMyGamepadKeyboardControl::RightStickLeft => state.right_stick.x -= 1.0,
                OhMyGamepadKeyboardControl::RightStickRight => state.right_stick.x += 1.0,
                OhMyGamepadKeyboardControl::South => state.buttons.south = 1.0,
                OhMyGamepadKeyboardControl::East => state.buttons.east = 1.0,
                OhMyGamepadKeyboardControl::West => state.buttons.west = 1.0,
                OhMyGamepadKeyboardControl::North => state.buttons.north = 1.0,
                OhMyGamepadKeyboardControl::L1 => state.buttons.l1 = 1.0,
                OhMyGamepadKeyboardControl::R1 => state.buttons.r1 = 1.0,
                OhMyGamepadKeyboardControl::L2 => state.left_trigger = 1.0,
                OhMyGamepadKeyboardControl::R2 => state.right_trigger = 1.0,
                OhMyGamepadKeyboardControl::L3 => state.buttons.l3 = 1.0,
                OhMyGamepadKeyboardControl::R3 => state.buttons.r3 = 1.0,
                OhMyGamepadKeyboardControl::View => state.buttons.view = 1.0,
                OhMyGamepadKeyboardControl::Menu => state.buttons.menu = 1.0,
                OhMyGamepadKeyboardControl::Home => state.buttons.home = 1.0,
                OhMyGamepadKeyboardControl::DpadUp => state.buttons.dpad_up = 1.0,
                OhMyGamepadKeyboardControl::DpadDown => state.buttons.dpad_down = 1.0,
                OhMyGamepadKeyboardControl::DpadLeft => state.buttons.dpad_left = 1.0,
                OhMyGamepadKeyboardControl::DpadRight => state.buttons.dpad_right = 1.0,
            }
        }

        state.left_stick = clamp_stick(state.left_stick);
        state.right_stick = clamp_stick(state.right_stick);
        state
    }

    pub fn apply_event(&mut self, event: OhMyGamepadKeyboardEvent) -> LogicalPadStateDto {
        if event.pressed {
            self.pressed_keys.insert(event.key);
        } else {
            self.pressed_keys.remove(&event.key);
        }
        self.state()
    }

    pub fn press_key(&mut self, key: OhMyGamepadKeyboardKey) -> LogicalPadStateDto {
        self.apply_event(OhMyGamepadKeyboardEvent { key, pressed: true })
    }

    pub fn release_key(&mut self, key: OhMyGamepadKeyboardKey) -> LogicalPadStateDto {
        self.apply_event(OhMyGamepadKeyboardEvent {
            key,
            pressed: false,
        })
    }

    pub fn toggle_key(&mut self, key: OhMyGamepadKeyboardKey) -> LogicalPadStateDto {
        if self.pressed_keys.contains(&key) {
            self.release_key(key)
        } else {
            self.press_key(key)
        }
    }

    pub fn clear(&mut self) -> LogicalPadStateDto {
        self.pressed_keys.clear();
        self.state()
    }

    pub fn sync_pressed_keys<I>(&mut self, keys: I) -> LogicalPadStateDto
    where
        I: IntoIterator<Item = OhMyGamepadKeyboardKey>,
    {
        self.pressed_keys = keys.into_iter().collect();
        self.state()
    }
}

fn clamp_stick(stick: LogicalStickDto) -> LogicalStickDto {
    LogicalStickDto {
        x: stick.x.clamp(-1.0, 1.0),
        y: stick.y.clamp(-1.0, 1.0),
    }
}
