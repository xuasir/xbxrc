use std::{
    collections::{HashMap, VecDeque},
    error::Error,
    fmt::{self, Display, Formatter},
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    time::{SystemTime, UNIX_EPOCH},
};

use gilrs::{
    ff::{BaseEffect, BaseEffectType, Effect, EffectBuilder, Repeat, Replay, Ticks},
    Axis, Button, Event, EventType, Gamepad, Gilrs, GilrsBuilder, PowerInfo,
};
use ohmygamepad_protocol::{
    OhMyGamepadCapabilityFlagsDto, OhMyGamepadConnectionKindDto, OhMyGamepadRumbleEffectDto,
};

use crate::{GilrsDeviceDescriptor, GilrsInputEvent, GilrsInputEventKind};

pub trait GilrsSource {
    fn next_event(&mut self) -> Option<GilrsInputEvent>;
}

#[derive(Clone, Debug)]
pub struct GilrsRumbleHandle {
    command_tx: Sender<GilrsRumbleCommand>,
}

impl GilrsRumbleHandle {
    pub fn play_rumble(
        &self,
        device_ids: Vec<String>,
        effect: OhMyGamepadRumbleEffectDto,
    ) -> Result<(), GilrsRumbleError> {
        self.command_tx
            .send(GilrsRumbleCommand::Play { device_ids, effect })
            .map_err(|_| GilrsRumbleError::CommandChannelClosed)
    }

    pub fn stop_rumble(&self, device_ids: Vec<String>) -> Result<(), GilrsRumbleError> {
        self.command_tx
            .send(GilrsRumbleCommand::Stop { device_ids })
            .map_err(|_| GilrsRumbleError::CommandChannelClosed)
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum GilrsRumbleError {
    CommandChannelClosed,
}

impl Display for GilrsRumbleError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::CommandChannelClosed => f.write_str("gilrs rumble command channel closed"),
        }
    }
}

impl Error for GilrsRumbleError {}

#[derive(Debug)]
enum GilrsRumbleCommand {
    Play {
        device_ids: Vec<String>,
        effect: OhMyGamepadRumbleEffectDto,
    },
    Stop {
        device_ids: Vec<String>,
    },
}

#[derive(Debug)]
pub struct GilrsSourceInitError {
    message: String,
}

impl GilrsSourceInitError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for GilrsSourceInitError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for GilrsSourceInitError {}

#[derive(Debug, Default)]
pub struct NoopGilrsSource;

impl GilrsSource for NoopGilrsSource {
    fn next_event(&mut self) -> Option<GilrsInputEvent> {
        None
    }
}

pub struct RealGilrsSource {
    gilrs: Gilrs,
    pending_events: VecDeque<GilrsInputEvent>,
    rumble_command_rx: Receiver<GilrsRumbleCommand>,
    active_rumble_effects: HashMap<String, Effect>,
}

impl RealGilrsSource {
    pub fn new() -> Result<(Self, Option<GilrsRumbleHandle>), GilrsSourceInitError> {
        let (gilrs, rumble_supported) = match GilrsBuilder::new().with_force_feedback(true).build()
        {
            Ok(gilrs) => (gilrs, true),
            // force feedback 在部分平台（尤其 macOS）会退回 NotImplemented。
            // 这里保留输入采集，但显式关闭 rumble backend，避免上层把无效请求误报成 accepted。
            Err(gilrs::Error::NotImplemented(gilrs)) => (gilrs, false),
            Err(error) => {
                return Err(GilrsSourceInitError::new(format!(
                    "failed to initialize gilrs: {error}"
                )));
            }
        };

        Ok(Self::from_gilrs(gilrs, rumble_supported))
    }

    fn from_gilrs(gilrs: Gilrs, rumble_supported: bool) -> (Self, Option<GilrsRumbleHandle>) {
        let (command_tx, rumble_command_rx) = mpsc::channel();
        let pending_events = gilrs
            .gamepads()
            .map(|(id, gamepad)| connected_event(id.to_string(), &gamepad, now_ms()))
            .collect();
        let rumble_handle = if rumble_supported {
            Some(GilrsRumbleHandle { command_tx })
        } else {
            None
        };

        (
            Self {
                gilrs,
                pending_events,
                rumble_command_rx,
                active_rumble_effects: HashMap::new(),
            },
            rumble_handle,
        )
    }

    fn drain_rumble_commands(&mut self) {
        loop {
            match self.rumble_command_rx.try_recv() {
                Ok(GilrsRumbleCommand::Play { device_ids, effect }) => {
                    self.play_rumble_on_devices(&device_ids, &effect);
                }
                Ok(GilrsRumbleCommand::Stop { device_ids }) => {
                    self.stop_rumble_on_devices(&device_ids);
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
    }

    fn play_rumble_on_devices(
        &mut self,
        device_ids: &[String],
        effect: &OhMyGamepadRumbleEffectDto,
    ) {
        let strong_magnitude =
            normalize_basic_rumble_magnitude(effect.strong_magnitude.max(effect.left_trigger));
        let weak_magnitude =
            normalize_basic_rumble_magnitude(effect.weak_magnitude.max(effect.right_trigger));
        if strong_magnitude == 0 && weak_magnitude == 0 {
            self.stop_rumble_on_devices(device_ids);
            return;
        }

        let replay = build_basic_rumble_replay(effect);
        let repeat = Repeat::For(total_rumble_duration(effect, replay));
        let resolved_devices = self.resolve_connected_gamepads(device_ids);
        for (device_id, gamepad_id) in resolved_devices {
            self.stop_rumble_on_devices(&[device_id.clone()]);

            let mut builder = EffectBuilder::new();
            if strong_magnitude > 0 {
                builder.add_effect(BaseEffect {
                    kind: BaseEffectType::Strong {
                        magnitude: strong_magnitude,
                    },
                    scheduling: replay,
                    ..Default::default()
                });
            }
            if weak_magnitude > 0 {
                builder.add_effect(BaseEffect {
                    kind: BaseEffectType::Weak {
                        magnitude: weak_magnitude,
                    },
                    scheduling: replay,
                    ..Default::default()
                });
            }
            builder.gamepads(&[gamepad_id]).repeat(repeat);

            if let Ok(effect_handle) = builder.finish(&mut self.gilrs) {
                if effect_handle.play().is_ok() {
                    self.active_rumble_effects.insert(device_id, effect_handle);
                }
            }
        }
    }

    fn stop_rumble_on_devices(&mut self, device_ids: &[String]) {
        for device_id in device_ids {
            let Some(effect) = self.active_rumble_effects.remove(device_id) else {
                continue;
            };
            let _ = effect.stop();
        }
    }

    fn resolve_connected_gamepads(&self, device_ids: &[String]) -> Vec<(String, gilrs::GamepadId)> {
        device_ids
            .iter()
            .filter_map(|device_id| {
                self.gilrs
                    .gamepads()
                    .find(|(id, gamepad)| gamepad.is_connected() && id.to_string() == *device_id)
                    .map(|(id, _)| (device_id.clone(), id))
            })
            .collect()
    }
}

impl GilrsSource for RealGilrsSource {
    fn next_event(&mut self) -> Option<GilrsInputEvent> {
        self.drain_rumble_commands();
        if let Some(event) = self.pending_events.pop_front() {
            return Some(event);
        }

        loop {
            let event = self.gilrs.next_event()?;
            if matches!(event.event, EventType::Disconnected) {
                self.stop_rumble_on_devices(&[event.id.to_string()]);
            }
            let translated = translate_event(&self.gilrs, event);
            if translated.is_empty() {
                continue;
            }

            let mut translated = translated.into_iter();
            let first = translated.next();
            self.pending_events.extend(translated);
            return first;
        }
    }
}

fn translate_event(gilrs: &Gilrs, event: Event) -> Vec<GilrsInputEvent> {
    let observed_at_ms = observed_at_ms(event.time);
    let device_id = event.id.to_string();
    let gamepad = gilrs.gamepad(event.id);
    let descriptor = descriptor_from_gamepad(device_id, &gamepad);

    match event.event {
        EventType::Connected => vec![GilrsInputEvent {
            device: descriptor,
            observed_at_ms,
            kind: GilrsInputEventKind::Connected,
        }],
        EventType::Disconnected => vec![GilrsInputEvent {
            device: descriptor,
            observed_at_ms,
            kind: GilrsInputEventKind::Disconnected,
        }],
        EventType::ButtonChanged(button, value, _) => map_button(button)
            .map(|index| {
                vec![GilrsInputEvent {
                    device: descriptor,
                    observed_at_ms,
                    kind: GilrsInputEventKind::ButtonChanged { index, value },
                }]
            })
            .unwrap_or_default(),
        EventType::ButtonPressed(button, _) | EventType::ButtonRepeated(button, _) => {
            map_button(button)
                .map(|index| {
                    vec![GilrsInputEvent {
                        device: descriptor,
                        observed_at_ms,
                        kind: GilrsInputEventKind::ButtonChanged { index, value: 1.0 },
                    }]
                })
                .unwrap_or_default()
        }
        EventType::ButtonReleased(button, _) => map_button(button)
            .map(|index| {
                vec![GilrsInputEvent {
                    device: descriptor,
                    observed_at_ms,
                    kind: GilrsInputEventKind::ButtonChanged { index, value: 0.0 },
                }]
            })
            .unwrap_or_default(),
        EventType::AxisChanged(axis, value, _) => {
            if let Some(index) = map_axis(axis) {
                return vec![GilrsInputEvent {
                    device: descriptor,
                    observed_at_ms,
                    kind: GilrsInputEventKind::AxisChanged { index, value },
                }];
            }
            if let Some(events) = dpad_axis_events(descriptor, observed_at_ms, axis, value) {
                return events;
            }
            Vec::new()
        }
        EventType::Dropped | EventType::ForceFeedbackEffectCompleted => Vec::new(),
        _ => Vec::new(),
    }
}

fn connected_event(
    device_id: String,
    gamepad: &Gamepad<'_>,
    observed_at_ms: u64,
) -> GilrsInputEvent {
    GilrsInputEvent {
        device: descriptor_from_gamepad(device_id, gamepad),
        observed_at_ms,
        kind: GilrsInputEventKind::Connected,
    }
}

fn descriptor_from_gamepad(device_id: String, gamepad: &Gamepad<'_>) -> GilrsDeviceDescriptor {
    let power_info = gamepad.power_info();

    GilrsDeviceDescriptor {
        device_id,
        name: gamepad.name().to_owned(),
        connection: connection_from_power_info(power_info),
        vendor_id: gamepad.vendor_id(),
        product_id: gamepad.product_id(),
        capabilities: OhMyGamepadCapabilityFlagsDto {
            basic_rumble: gamepad.is_ff_supported(),
            advanced_haptics: false,
            battery: has_battery(power_info),
        },
    }
}

fn connection_from_power_info(power_info: PowerInfo) -> Option<OhMyGamepadConnectionKindDto> {
    match power_info {
        PowerInfo::Wired => Some(OhMyGamepadConnectionKindDto::Usb),
        PowerInfo::Charging(_) | PowerInfo::Charged | PowerInfo::Discharging(_) => {
            Some(OhMyGamepadConnectionKindDto::Unknown)
        }
        PowerInfo::Unknown => None,
    }
}

fn has_battery(power_info: PowerInfo) -> bool {
    matches!(
        power_info,
        PowerInfo::Charging(_) | PowerInfo::Charged | PowerInfo::Discharging(_)
    )
}

fn map_button(button: Button) -> Option<usize> {
    match button {
        Button::South => Some(0),
        Button::East => Some(1),
        Button::West => Some(2),
        Button::North => Some(3),
        Button::LeftTrigger => Some(4),
        Button::RightTrigger => Some(5),
        Button::LeftTrigger2 => Some(6),
        Button::RightTrigger2 => Some(7),
        Button::Select => Some(8),
        Button::Start => Some(9),
        Button::LeftThumb => Some(10),
        Button::RightThumb => Some(11),
        Button::DPadUp => Some(12),
        Button::DPadDown => Some(13),
        Button::DPadLeft => Some(14),
        Button::DPadRight => Some(15),
        Button::Mode => Some(16),
        Button::C | Button::Z | Button::Unknown => None,
    }
}

fn map_axis(axis: Axis) -> Option<usize> {
    match axis {
        Axis::LeftStickX => Some(0),
        Axis::LeftStickY => Some(1),
        Axis::RightStickX => Some(2),
        Axis::RightStickY => Some(3),
        Axis::LeftZ => Some(4),
        Axis::RightZ => Some(5),
        Axis::DPadX | Axis::DPadY | Axis::Unknown => None,
    }
}

fn dpad_axis_events(
    descriptor: GilrsDeviceDescriptor,
    observed_at_ms: u64,
    axis: Axis,
    value: f32,
) -> Option<Vec<GilrsInputEvent>> {
    let (negative_index, positive_index) = match axis {
        Axis::DPadX => (14, 15),
        Axis::DPadY => (12, 13),
        _ => return None,
    };

    let negative_value = if value < 0.0 { value.abs() } else { 0.0 };
    let positive_value = if value > 0.0 { value.abs() } else { 0.0 };

    Some(vec![
        GilrsInputEvent {
            device: descriptor.clone(),
            observed_at_ms,
            kind: GilrsInputEventKind::ButtonChanged {
                index: negative_index,
                value: negative_value,
            },
        },
        GilrsInputEvent {
            device: descriptor,
            observed_at_ms,
            kind: GilrsInputEventKind::ButtonChanged {
                index: positive_index,
                value: positive_value,
            },
        },
    ])
}

fn observed_at_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn now_ms() -> u64 {
    observed_at_ms(SystemTime::now())
}

fn normalize_basic_rumble_magnitude(value: f32) -> u16 {
    (value.clamp(0.0, 1.0) * u16::MAX as f32).round() as u16
}

fn build_basic_rumble_replay(effect: &OhMyGamepadRumbleEffectDto) -> Replay {
    Replay {
        after: Ticks::from_ms(effect.start_delay_ms as u32),
        play_for: Ticks::from_ms(effect.duration_ms.max(1) as u32),
        with_delay: Ticks::from_ms(effect.start_delay_ms as u32),
    }
}

fn total_rumble_duration(effect: &OhMyGamepadRumbleEffectDto, replay: Replay) -> Ticks {
    replay.after + replay.play_for + replay.dur() * u32::from(effect.repeat)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use gilrs::{Axis, Button};
    use ohmygamepad_protocol::OhMyGamepadRumbleEffectDto;

    use super::{
        build_basic_rumble_replay, dpad_axis_events, map_axis, map_button,
        normalize_basic_rumble_magnitude, observed_at_ms, total_rumble_duration,
    };
    use crate::{GilrsDeviceDescriptor, GilrsInputEventKind};

    #[test]
    fn button_mapping_matches_input_core_default_layout() {
        assert_eq!(map_button(Button::South), Some(0));
        assert_eq!(map_button(Button::LeftTrigger2), Some(6));
        assert_eq!(map_button(Button::Mode), Some(16));
        assert_eq!(map_button(Button::C), None);
    }

    #[test]
    fn axis_mapping_matches_input_core_default_layout() {
        assert_eq!(map_axis(Axis::LeftStickX), Some(0));
        assert_eq!(map_axis(Axis::RightZ), Some(5));
        assert_eq!(map_axis(Axis::DPadX), None);
    }

    #[test]
    fn dpad_axis_expands_to_opposite_button_pair() {
        let events = dpad_axis_events(
            GilrsDeviceDescriptor {
                device_id: "gilrs:1".to_owned(),
                name: "pad".to_owned(),
                ..GilrsDeviceDescriptor::default()
            },
            10,
            Axis::DPadX,
            0.7,
        )
        .expect("dpad x should map to button pair");

        assert_eq!(events.len(), 2);
        assert!(matches!(
            events[0].kind,
            GilrsInputEventKind::ButtonChanged {
                index: 14,
                value: 0.0
            }
        ));
        assert!(matches!(
            events[1].kind,
            GilrsInputEventKind::ButtonChanged {
                index: 15,
                value: 0.7
            }
        ));
    }

    #[test]
    fn system_time_is_converted_to_milliseconds() {
        let time = UNIX_EPOCH + Duration::from_millis(1234);
        assert_eq!(observed_at_ms(time), 1234);
    }

    #[test]
    fn basic_rumble_magnitude_clamps_to_motor_range() {
        assert_eq!(normalize_basic_rumble_magnitude(-1.0), 0);
        assert_eq!(normalize_basic_rumble_magnitude(0.5), 32768);
        assert_eq!(normalize_basic_rumble_magnitude(1.5), u16::MAX);
    }

    #[test]
    fn replay_duration_respects_delay_and_repeat() {
        let effect = OhMyGamepadRumbleEffectDto {
            start_delay_ms: 50,
            duration_ms: 120,
            repeat: 2,
            ..OhMyGamepadRumbleEffectDto::default()
        };
        let replay = build_basic_rumble_replay(&effect);

        assert_eq!(replay.after, gilrs::ff::Ticks::from_ms(50));
        assert_eq!(replay.play_for, gilrs::ff::Ticks::from_ms(120));
        assert_eq!(
            total_rumble_duration(&effect, replay),
            gilrs::ff::Ticks::from_ms(50) + gilrs::ff::Ticks::from_ms(120) + replay.dur() * 2
        );
    }
}
