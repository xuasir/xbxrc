use std::{collections::BTreeSet, panic::AssertUnwindSafe, time::Duration};

use device_query::DeviceState;
#[cfg(not(target_os = "macos"))]
use device_query::{DeviceQuery, Keycode};
use ohmygamepad_protocol::LogicalPadStateDto;

#[cfg(target_os = "macos")]
use crate::macos_keyboard_hid;
use crate::{
    keyboard::{OhMyGamepadKeyboardKey, OhMyGamepadKeyboardMapper, OhMyGamepadKeyboardMapping},
    OhMyGamepadService, OhMyGamepadServiceError,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OhMyGamepadDesktopKeyboardListenerConfig {
    pub poll_interval: Duration,
    pub mapping: OhMyGamepadKeyboardMapping,
}

impl Default for OhMyGamepadDesktopKeyboardListenerConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(8),
            mapping: OhMyGamepadKeyboardMapping::default(),
        }
    }
}

pub struct OhMyGamepadDesktopKeyboardListener {
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    device_state: DeviceState,
    mapper: OhMyGamepadKeyboardMapper,
    poll_interval: Duration,
    last_pressed_keys: BTreeSet<OhMyGamepadKeyboardKey>,
}

impl std::fmt::Debug for OhMyGamepadDesktopKeyboardListener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OhMyGamepadDesktopKeyboardListener")
            .field("poll_interval", &self.poll_interval)
            .field("last_pressed_keys", &self.last_pressed_keys)
            .finish()
    }
}

impl Default for OhMyGamepadDesktopKeyboardListener {
    fn default() -> Self {
        Self::new(OhMyGamepadDesktopKeyboardListenerConfig::default())
    }
}

impl OhMyGamepadDesktopKeyboardListener {
    pub fn new(config: OhMyGamepadDesktopKeyboardListenerConfig) -> Self {
        Self::try_new(config).expect("desktop keyboard listener init")
    }

    pub fn try_new(config: OhMyGamepadDesktopKeyboardListenerConfig) -> Result<Self, String> {
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let device_state = std::panic::catch_unwind(AssertUnwindSafe(DeviceState::new));
        std::panic::set_hook(previous_hook);
        let device_state = device_state.map_err(|_| {
            "desktop keyboard listener requires OS accessibility permission".to_owned()
        })?;
        Ok(Self {
            device_state,
            mapper: OhMyGamepadKeyboardMapper::new(config.mapping),
            poll_interval: config.poll_interval,
            last_pressed_keys: BTreeSet::new(),
        })
    }

    pub fn poll_interval(&self) -> Duration {
        self.poll_interval
    }

    pub fn set_mapping(&mut self, mapping: OhMyGamepadKeyboardMapping) -> LogicalPadStateDto {
        self.mapper.replace_mapping(mapping)
    }

    pub fn mapping(&self) -> &OhMyGamepadKeyboardMapping {
        self.mapper.mapping()
    }

    pub fn pressed_keys(&self) -> &BTreeSet<OhMyGamepadKeyboardKey> {
        &self.last_pressed_keys
    }

    pub fn is_pressed(&self, key: OhMyGamepadKeyboardKey) -> bool {
        self.last_pressed_keys.contains(&key)
    }

    pub fn poll_state(&mut self) -> LogicalPadStateDto {
        let pressed_keys = self.read_pressed_keys();
        self.last_pressed_keys = pressed_keys.clone();
        self.mapper.sync_pressed_keys(pressed_keys)
    }

    pub fn submit_to_service(
        &mut self,
        service: &OhMyGamepadService,
    ) -> Result<LogicalPadStateDto, OhMyGamepadServiceError> {
        let state = self.poll_state();
        service.submit_keyboard_state(state)?;
        Ok(state)
    }

    fn read_pressed_keys(&self) -> BTreeSet<OhMyGamepadKeyboardKey> {
        #[cfg(target_os = "macos")]
        {
            return macos_keyboard_hid::read_pressed_keys_hid();
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.device_state
                .get_keys()
                .into_iter()
                .filter_map(map_keycode)
                .collect()
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn map_keycode(keycode: Keycode) -> Option<OhMyGamepadKeyboardKey> {
    match keycode {
        Keycode::A => Some(OhMyGamepadKeyboardKey::KeyA),
        Keycode::B => Some(OhMyGamepadKeyboardKey::KeyB),
        Keycode::C => Some(OhMyGamepadKeyboardKey::KeyC),
        Keycode::D => Some(OhMyGamepadKeyboardKey::KeyD),
        Keycode::E => Some(OhMyGamepadKeyboardKey::KeyE),
        Keycode::F => Some(OhMyGamepadKeyboardKey::KeyF),
        Keycode::G => Some(OhMyGamepadKeyboardKey::KeyG),
        Keycode::H => Some(OhMyGamepadKeyboardKey::KeyH),
        Keycode::I => Some(OhMyGamepadKeyboardKey::KeyI),
        Keycode::J => Some(OhMyGamepadKeyboardKey::KeyJ),
        Keycode::K => Some(OhMyGamepadKeyboardKey::KeyK),
        Keycode::L => Some(OhMyGamepadKeyboardKey::KeyL),
        Keycode::M => Some(OhMyGamepadKeyboardKey::KeyM),
        Keycode::N => Some(OhMyGamepadKeyboardKey::KeyN),
        Keycode::O => Some(OhMyGamepadKeyboardKey::KeyO),
        Keycode::P => Some(OhMyGamepadKeyboardKey::KeyP),
        Keycode::Q => Some(OhMyGamepadKeyboardKey::KeyQ),
        Keycode::R => Some(OhMyGamepadKeyboardKey::KeyR),
        Keycode::S => Some(OhMyGamepadKeyboardKey::KeyS),
        Keycode::T => Some(OhMyGamepadKeyboardKey::KeyT),
        Keycode::U => Some(OhMyGamepadKeyboardKey::KeyU),
        Keycode::V => Some(OhMyGamepadKeyboardKey::KeyV),
        Keycode::W => Some(OhMyGamepadKeyboardKey::KeyW),
        Keycode::X => Some(OhMyGamepadKeyboardKey::KeyX),
        Keycode::Y => Some(OhMyGamepadKeyboardKey::KeyY),
        Keycode::Z => Some(OhMyGamepadKeyboardKey::KeyZ),
        Keycode::Key0 => Some(OhMyGamepadKeyboardKey::Digit0),
        Keycode::Key1 => Some(OhMyGamepadKeyboardKey::Digit1),
        Keycode::Key2 => Some(OhMyGamepadKeyboardKey::Digit2),
        Keycode::Key3 => Some(OhMyGamepadKeyboardKey::Digit3),
        Keycode::Key4 => Some(OhMyGamepadKeyboardKey::Digit4),
        Keycode::Key5 => Some(OhMyGamepadKeyboardKey::Digit5),
        Keycode::Key6 => Some(OhMyGamepadKeyboardKey::Digit6),
        Keycode::Key7 => Some(OhMyGamepadKeyboardKey::Digit7),
        Keycode::Key8 => Some(OhMyGamepadKeyboardKey::Digit8),
        Keycode::Key9 => Some(OhMyGamepadKeyboardKey::Digit9),
        Keycode::Enter => Some(OhMyGamepadKeyboardKey::Enter),
        Keycode::Tab => Some(OhMyGamepadKeyboardKey::Tab),
        Keycode::Escape => Some(OhMyGamepadKeyboardKey::Escape),
        Keycode::Space => Some(OhMyGamepadKeyboardKey::Space),
        Keycode::Up => Some(OhMyGamepadKeyboardKey::ArrowUp),
        Keycode::Down => Some(OhMyGamepadKeyboardKey::ArrowDown),
        Keycode::Left => Some(OhMyGamepadKeyboardKey::ArrowLeft),
        Keycode::Right => Some(OhMyGamepadKeyboardKey::ArrowRight),
        _ => None,
    }
}
