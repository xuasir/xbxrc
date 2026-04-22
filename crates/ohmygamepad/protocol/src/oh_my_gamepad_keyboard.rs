use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OhMyGamepadKeyboardKeyDto {
    KeyA,
    KeyB,
    KeyC,
    KeyD,
    KeyE,
    KeyF,
    KeyG,
    KeyH,
    KeyI,
    KeyJ,
    KeyK,
    KeyL,
    KeyM,
    KeyN,
    KeyO,
    KeyP,
    KeyQ,
    KeyR,
    KeyS,
    KeyT,
    KeyU,
    KeyV,
    KeyW,
    KeyX,
    KeyY,
    KeyZ,
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    Enter,
    Tab,
    Escape,
    Space,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
}

impl OhMyGamepadKeyboardKeyDto {
    pub fn from_char(value: char) -> Option<Self> {
        match value.to_ascii_lowercase() {
            'a' => Some(Self::KeyA),
            'b' => Some(Self::KeyB),
            'c' => Some(Self::KeyC),
            'd' => Some(Self::KeyD),
            'e' => Some(Self::KeyE),
            'f' => Some(Self::KeyF),
            'g' => Some(Self::KeyG),
            'h' => Some(Self::KeyH),
            'i' => Some(Self::KeyI),
            'j' => Some(Self::KeyJ),
            'k' => Some(Self::KeyK),
            'l' => Some(Self::KeyL),
            'm' => Some(Self::KeyM),
            'n' => Some(Self::KeyN),
            'o' => Some(Self::KeyO),
            'p' => Some(Self::KeyP),
            'q' => Some(Self::KeyQ),
            'r' => Some(Self::KeyR),
            's' => Some(Self::KeyS),
            't' => Some(Self::KeyT),
            'u' => Some(Self::KeyU),
            'v' => Some(Self::KeyV),
            'w' => Some(Self::KeyW),
            'x' => Some(Self::KeyX),
            'y' => Some(Self::KeyY),
            'z' => Some(Self::KeyZ),
            '0' => Some(Self::Digit0),
            '1' => Some(Self::Digit1),
            '2' => Some(Self::Digit2),
            '3' => Some(Self::Digit3),
            '4' => Some(Self::Digit4),
            '5' => Some(Self::Digit5),
            '6' => Some(Self::Digit6),
            '7' => Some(Self::Digit7),
            '8' => Some(Self::Digit8),
            '9' => Some(Self::Digit9),
            ' ' => Some(Self::Space),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OhMyGamepadKeyboardControlDto {
    LeftStickUp,
    LeftStickDown,
    LeftStickLeft,
    LeftStickRight,
    RightStickUp,
    RightStickDown,
    RightStickLeft,
    RightStickRight,
    South,
    East,
    West,
    North,
    L1,
    R1,
    L2,
    R2,
    L3,
    R3,
    View,
    Menu,
    Home,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OhMyGamepadKeyboardBindingDto {
    pub key: OhMyGamepadKeyboardKeyDto,
    pub control: OhMyGamepadKeyboardControlDto,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OhMyGamepadKeyboardMappingDto {
    pub bindings: Vec<OhMyGamepadKeyboardBindingDto>,
}

impl Default for OhMyGamepadKeyboardMappingDto {
    fn default() -> Self {
        // 先提供一套桌面调试友好的默认映射，后续宿主层可以覆盖。
        Self {
            bindings: vec![
                bind(
                    OhMyGamepadKeyboardKeyDto::KeyW,
                    OhMyGamepadKeyboardControlDto::LeftStickUp,
                ),
                bind(
                    OhMyGamepadKeyboardKeyDto::KeyS,
                    OhMyGamepadKeyboardControlDto::LeftStickDown,
                ),
                bind(
                    OhMyGamepadKeyboardKeyDto::KeyA,
                    OhMyGamepadKeyboardControlDto::LeftStickLeft,
                ),
                bind(
                    OhMyGamepadKeyboardKeyDto::KeyD,
                    OhMyGamepadKeyboardControlDto::LeftStickRight,
                ),
                bind(
                    OhMyGamepadKeyboardKeyDto::KeyT,
                    OhMyGamepadKeyboardControlDto::RightStickUp,
                ),
                bind(
                    OhMyGamepadKeyboardKeyDto::KeyG,
                    OhMyGamepadKeyboardControlDto::RightStickDown,
                ),
                bind(
                    OhMyGamepadKeyboardKeyDto::KeyF,
                    OhMyGamepadKeyboardControlDto::RightStickLeft,
                ),
                bind(
                    OhMyGamepadKeyboardKeyDto::KeyH,
                    OhMyGamepadKeyboardControlDto::RightStickRight,
                ),
                bind(
                    OhMyGamepadKeyboardKeyDto::KeyJ,
                    OhMyGamepadKeyboardControlDto::South,
                ),
                bind(
                    OhMyGamepadKeyboardKeyDto::KeyK,
                    OhMyGamepadKeyboardControlDto::East,
                ),
                bind(
                    OhMyGamepadKeyboardKeyDto::KeyU,
                    OhMyGamepadKeyboardControlDto::West,
                ),
                bind(
                    OhMyGamepadKeyboardKeyDto::KeyI,
                    OhMyGamepadKeyboardControlDto::North,
                ),
                bind(
                    OhMyGamepadKeyboardKeyDto::Digit1,
                    OhMyGamepadKeyboardControlDto::L1,
                ),
                bind(
                    OhMyGamepadKeyboardKeyDto::Digit2,
                    OhMyGamepadKeyboardControlDto::R1,
                ),
                bind(
                    OhMyGamepadKeyboardKeyDto::Digit3,
                    OhMyGamepadKeyboardControlDto::L2,
                ),
                bind(
                    OhMyGamepadKeyboardKeyDto::Digit4,
                    OhMyGamepadKeyboardControlDto::R2,
                ),
                bind(
                    OhMyGamepadKeyboardKeyDto::KeyZ,
                    OhMyGamepadKeyboardControlDto::L3,
                ),
                bind(
                    OhMyGamepadKeyboardKeyDto::KeyX,
                    OhMyGamepadKeyboardControlDto::R3,
                ),
                bind(
                    OhMyGamepadKeyboardKeyDto::Tab,
                    OhMyGamepadKeyboardControlDto::View,
                ),
                bind(
                    OhMyGamepadKeyboardKeyDto::Digit7,
                    OhMyGamepadKeyboardControlDto::View,
                ),
                bind(
                    OhMyGamepadKeyboardKeyDto::Enter,
                    OhMyGamepadKeyboardControlDto::Menu,
                ),
                bind(
                    OhMyGamepadKeyboardKeyDto::Digit8,
                    OhMyGamepadKeyboardControlDto::Menu,
                ),
                bind(
                    OhMyGamepadKeyboardKeyDto::Digit9,
                    OhMyGamepadKeyboardControlDto::Home,
                ),
                bind(
                    OhMyGamepadKeyboardKeyDto::ArrowUp,
                    OhMyGamepadKeyboardControlDto::DpadUp,
                ),
                bind(
                    OhMyGamepadKeyboardKeyDto::ArrowDown,
                    OhMyGamepadKeyboardControlDto::DpadDown,
                ),
                bind(
                    OhMyGamepadKeyboardKeyDto::ArrowLeft,
                    OhMyGamepadKeyboardControlDto::DpadLeft,
                ),
                bind(
                    OhMyGamepadKeyboardKeyDto::ArrowRight,
                    OhMyGamepadKeyboardControlDto::DpadRight,
                ),
            ],
        }
    }
}

impl OhMyGamepadKeyboardMappingDto {
    pub fn bindings(&self) -> &[OhMyGamepadKeyboardBindingDto] {
        &self.bindings
    }

    pub fn add_binding(&mut self, binding: OhMyGamepadKeyboardBindingDto) {
        self.bindings.push(binding);
    }

    pub fn clear_bindings(&mut self) {
        self.bindings.clear();
    }

    pub fn remove_bindings_for_key(&mut self, key: OhMyGamepadKeyboardKeyDto) {
        self.bindings.retain(|binding| binding.key != key);
    }

    pub fn replace_binding(
        &mut self,
        key: OhMyGamepadKeyboardKeyDto,
        control: OhMyGamepadKeyboardControlDto,
    ) {
        self.remove_bindings_for_key(key);
        self.add_binding(bind(key, control));
    }
}

fn bind(
    key: OhMyGamepadKeyboardKeyDto,
    control: OhMyGamepadKeyboardControlDto,
) -> OhMyGamepadKeyboardBindingDto {
    OhMyGamepadKeyboardBindingDto { key, control }
}
