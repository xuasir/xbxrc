use std::collections::BTreeSet;

use crate::keyboard::OhMyGamepadKeyboardKey;

#[derive(Clone, Copy)]
#[repr(i32)]
enum CGEventSourceStateId {
    HidSystem = 1,
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceKeyState(state: CGEventSourceStateId, keycode: u16) -> bool;
}

#[inline]
fn key_down_hid(keycode: u16) -> bool {
    unsafe { CGEventSourceKeyState(CGEventSourceStateId::HidSystem, keycode) }
}

fn cg_keycode(key: OhMyGamepadKeyboardKey) -> u16 {
    match key {
        OhMyGamepadKeyboardKey::KeyA => 0x00,
        OhMyGamepadKeyboardKey::KeyS => 0x01,
        OhMyGamepadKeyboardKey::KeyD => 0x02,
        OhMyGamepadKeyboardKey::KeyF => 0x03,
        OhMyGamepadKeyboardKey::KeyH => 0x04,
        OhMyGamepadKeyboardKey::KeyG => 0x05,
        OhMyGamepadKeyboardKey::KeyZ => 0x06,
        OhMyGamepadKeyboardKey::KeyX => 0x07,
        OhMyGamepadKeyboardKey::KeyC => 0x08,
        OhMyGamepadKeyboardKey::KeyV => 0x09,
        OhMyGamepadKeyboardKey::KeyB => 0x0B,
        OhMyGamepadKeyboardKey::KeyQ => 0x0C,
        OhMyGamepadKeyboardKey::KeyW => 0x0D,
        OhMyGamepadKeyboardKey::KeyE => 0x0E,
        OhMyGamepadKeyboardKey::KeyR => 0x0F,
        OhMyGamepadKeyboardKey::KeyY => 0x10,
        OhMyGamepadKeyboardKey::KeyT => 0x11,
        OhMyGamepadKeyboardKey::Digit1 => 0x12,
        OhMyGamepadKeyboardKey::Digit2 => 0x13,
        OhMyGamepadKeyboardKey::Digit3 => 0x14,
        OhMyGamepadKeyboardKey::Digit4 => 0x15,
        OhMyGamepadKeyboardKey::Digit6 => 0x16,
        OhMyGamepadKeyboardKey::Digit5 => 0x17,
        OhMyGamepadKeyboardKey::Digit9 => 0x19,
        OhMyGamepadKeyboardKey::Digit7 => 0x1A,
        OhMyGamepadKeyboardKey::Digit8 => 0x1C,
        OhMyGamepadKeyboardKey::Digit0 => 0x1D,
        OhMyGamepadKeyboardKey::KeyO => 0x1F,
        OhMyGamepadKeyboardKey::KeyU => 0x20,
        OhMyGamepadKeyboardKey::KeyI => 0x22,
        OhMyGamepadKeyboardKey::KeyP => 0x23,
        OhMyGamepadKeyboardKey::KeyL => 0x25,
        OhMyGamepadKeyboardKey::KeyJ => 0x26,
        OhMyGamepadKeyboardKey::KeyK => 0x28,
        OhMyGamepadKeyboardKey::KeyN => 0x2D,
        OhMyGamepadKeyboardKey::KeyM => 0x2E,
        OhMyGamepadKeyboardKey::Enter => 0x24,
        OhMyGamepadKeyboardKey::Tab => 0x30,
        OhMyGamepadKeyboardKey::Space => 0x31,
        OhMyGamepadKeyboardKey::Escape => 0x35,
        OhMyGamepadKeyboardKey::ArrowLeft => 0x7B,
        OhMyGamepadKeyboardKey::ArrowRight => 0x7C,
        OhMyGamepadKeyboardKey::ArrowDown => 0x7D,
        OhMyGamepadKeyboardKey::ArrowUp => 0x7E,
    }
}

const KEYS: &[OhMyGamepadKeyboardKey] = &[
    OhMyGamepadKeyboardKey::KeyA,
    OhMyGamepadKeyboardKey::KeyB,
    OhMyGamepadKeyboardKey::KeyC,
    OhMyGamepadKeyboardKey::KeyD,
    OhMyGamepadKeyboardKey::KeyE,
    OhMyGamepadKeyboardKey::KeyF,
    OhMyGamepadKeyboardKey::KeyG,
    OhMyGamepadKeyboardKey::KeyH,
    OhMyGamepadKeyboardKey::KeyI,
    OhMyGamepadKeyboardKey::KeyJ,
    OhMyGamepadKeyboardKey::KeyK,
    OhMyGamepadKeyboardKey::KeyL,
    OhMyGamepadKeyboardKey::KeyM,
    OhMyGamepadKeyboardKey::KeyN,
    OhMyGamepadKeyboardKey::KeyO,
    OhMyGamepadKeyboardKey::KeyP,
    OhMyGamepadKeyboardKey::KeyQ,
    OhMyGamepadKeyboardKey::KeyR,
    OhMyGamepadKeyboardKey::KeyS,
    OhMyGamepadKeyboardKey::KeyT,
    OhMyGamepadKeyboardKey::KeyU,
    OhMyGamepadKeyboardKey::KeyV,
    OhMyGamepadKeyboardKey::KeyW,
    OhMyGamepadKeyboardKey::KeyX,
    OhMyGamepadKeyboardKey::KeyY,
    OhMyGamepadKeyboardKey::KeyZ,
    OhMyGamepadKeyboardKey::Digit0,
    OhMyGamepadKeyboardKey::Digit1,
    OhMyGamepadKeyboardKey::Digit2,
    OhMyGamepadKeyboardKey::Digit3,
    OhMyGamepadKeyboardKey::Digit4,
    OhMyGamepadKeyboardKey::Digit5,
    OhMyGamepadKeyboardKey::Digit6,
    OhMyGamepadKeyboardKey::Digit7,
    OhMyGamepadKeyboardKey::Digit8,
    OhMyGamepadKeyboardKey::Digit9,
    OhMyGamepadKeyboardKey::Enter,
    OhMyGamepadKeyboardKey::Tab,
    OhMyGamepadKeyboardKey::Escape,
    OhMyGamepadKeyboardKey::Space,
    OhMyGamepadKeyboardKey::ArrowUp,
    OhMyGamepadKeyboardKey::ArrowDown,
    OhMyGamepadKeyboardKey::ArrowLeft,
    OhMyGamepadKeyboardKey::ArrowRight,
];

pub fn read_pressed_keys_hid() -> BTreeSet<OhMyGamepadKeyboardKey> {
    let mut out = BTreeSet::new();
    for &key in KEYS {
        if key_down_hid(cg_keycode(key)) {
            out.insert(key);
        }
    }
    out
}
