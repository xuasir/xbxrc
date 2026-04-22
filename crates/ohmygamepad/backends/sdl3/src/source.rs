use std::{
    collections::HashMap,
    error::Error,
    fmt::{self, Display, Formatter},
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use ohmygamepad_protocol::{
    OhMyGamepadCapabilityFlagsDto, OhMyGamepadConnectionKindDto, OhMyGamepadDeviceTypeDto,
    OhMyGamepadPowerStateDto, OhMyGamepadRumbleEffectDto,
};
use sdl3::{
    event::Event,
    gamepad::{Axis, Button, Gamepad, GamepadType},
    joystick::{ConnectionState, JoystickId, PowerInfo, PowerLevel},
};

use crate::{Sdl3DeviceDescriptor, Sdl3InputEvent, Sdl3InputEventKind};

pub trait Sdl3Source {
    fn next_event(&mut self) -> Option<Sdl3InputEvent>;
}

#[derive(Clone, Debug)]
pub struct Sdl3RumbleHandle {
    command_tx: Sender<Sdl3RumbleCommand>,
}

impl Sdl3RumbleHandle {
    pub fn play_rumble(
        &self,
        device_ids: Vec<String>,
        effect: OhMyGamepadRumbleEffectDto,
    ) -> Result<(), Sdl3RumbleError> {
        self.command_tx
            .send(Sdl3RumbleCommand::Play { device_ids, effect })
            .map_err(|_| Sdl3RumbleError::CommandChannelClosed)
    }

    pub fn stop_rumble(&self, device_ids: Vec<String>) -> Result<(), Sdl3RumbleError> {
        self.command_tx
            .send(Sdl3RumbleCommand::Stop { device_ids })
            .map_err(|_| Sdl3RumbleError::CommandChannelClosed)
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum Sdl3RumbleError {
    CommandChannelClosed,
}

impl Display for Sdl3RumbleError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::CommandChannelClosed => f.write_str("sdl3 rumble command channel closed"),
        }
    }
}

impl Error for Sdl3RumbleError {}

#[derive(Debug)]
enum Sdl3RumbleCommand {
    Play {
        device_ids: Vec<String>,
        effect: OhMyGamepadRumbleEffectDto,
    },
    Stop {
        device_ids: Vec<String>,
    },
}

#[derive(Debug)]
pub struct Sdl3SourceInitError {
    message: String,
}

impl Sdl3SourceInitError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for Sdl3SourceInitError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for Sdl3SourceInitError {}

#[derive(Debug, Default)]
pub struct NoopSdl3Source;

impl Sdl3Source for NoopSdl3Source {
    fn next_event(&mut self) -> Option<Sdl3InputEvent> {
        None
    }
}

#[derive(Debug)]
pub struct RealSdl3Source {
    event_rx: Receiver<Sdl3InputEvent>,
}

impl RealSdl3Source {
    pub fn new() -> Result<(Self, Option<Sdl3RumbleHandle>), Sdl3SourceInitError> {
        let (event_tx, event_rx) = mpsc::channel();
        let (command_tx, command_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();

        thread::Builder::new()
            .name("ohmygamepad-sdl3-source".to_owned())
            .spawn(move || run_sdl3_source_thread(event_tx, command_rx, ready_tx))
            .map_err(|error| {
                Sdl3SourceInitError::new(format!("spawn sdl3 source thread: {error}"))
            })?;

        match ready_rx.recv() {
            Ok(Ok(())) => Ok((Self { event_rx }, Some(Sdl3RumbleHandle { command_tx }))),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(Sdl3SourceInitError::new(
                "sdl3 source thread exited before initialization completed",
            )),
        }
    }
}

impl Sdl3Source for RealSdl3Source {
    fn next_event(&mut self) -> Option<Sdl3InputEvent> {
        self.event_rx.try_recv().ok()
    }
}

fn run_sdl3_source_thread(
    event_tx: Sender<Sdl3InputEvent>,
    command_rx: Receiver<Sdl3RumbleCommand>,
    ready_tx: Sender<Result<(), Sdl3SourceInitError>>,
) {
    let sdl = match sdl3::init() {
        Ok(value) => value,
        Err(error) => {
            let _ = ready_tx.send(Err(Sdl3SourceInitError::new(format!(
                "init sdl3 failed: {error}"
            ))));
            return;
        }
    };
    let gamepad_subsystem = match sdl.gamepad() {
        Ok(value) => value,
        Err(error) => {
            let _ = ready_tx.send(Err(Sdl3SourceInitError::new(format!(
                "init sdl3 gamepad subsystem failed: {error}"
            ))));
            return;
        }
    };
    let mut event_pump = match sdl.event_pump() {
        Ok(value) => value,
        Err(error) => {
            let _ = ready_tx.send(Err(Sdl3SourceInitError::new(format!(
                "init sdl3 event pump failed: {error}"
            ))));
            return;
        }
    };

    gamepad_subsystem.set_events_processing_state(true);

    let mut opened_gamepads = HashMap::new();
    if let Ok(gamepad_ids) = gamepad_subsystem.gamepads() {
        for gamepad_id in gamepad_ids {
            if let Some((device_id, opened)) = open_gamepad(&gamepad_subsystem, gamepad_id) {
                if !send_event(
                    &event_tx,
                    Sdl3InputEvent {
                        device: opened.descriptor.clone(),
                        observed_at_ms: now_ms(),
                        kind: Sdl3InputEventKind::Connected,
                    },
                ) {
                    return;
                }
                opened_gamepads.insert(device_id, opened);
            }
        }
    }

    if ready_tx.send(Ok(())).is_err() {
        return;
    }

    loop {
        drain_rumble_commands(&command_rx, &mut opened_gamepads);
        gamepad_subsystem.update();

        let mut received_event = false;
        while let Some(event) = event_pump.poll_event() {
            received_event = true;
            if !handle_sdl_event(&event_tx, &gamepad_subsystem, &mut opened_gamepads, event) {
                return;
            }
        }

        if !received_event {
            thread::sleep(Duration::from_millis(2));
        }
    }
}

struct OpenedSdl3Gamepad {
    gamepad: Gamepad,
    descriptor: Sdl3DeviceDescriptor,
}

fn handle_sdl_event(
    event_tx: &Sender<Sdl3InputEvent>,
    gamepad_subsystem: &sdl3::GamepadSubsystem,
    opened_gamepads: &mut HashMap<String, OpenedSdl3Gamepad>,
    event: Event,
) -> bool {
    match event {
        Event::ControllerDeviceAdded { which, .. } => {
            if let Some((device_id, opened)) = open_gamepad_from_event(gamepad_subsystem, which) {
                let descriptor = opened.descriptor.clone();
                opened_gamepads.insert(device_id, opened);
                send_event(
                    event_tx,
                    Sdl3InputEvent {
                        device: descriptor,
                        observed_at_ms: now_ms(),
                        kind: Sdl3InputEventKind::Connected,
                    },
                )
            } else {
                true
            }
        }
        Event::ControllerDeviceRemoved { which, .. } => {
            let device_id = joystick_instance_id_to_device_id(which);
            let Some(opened) = opened_gamepads.remove(&device_id) else {
                return true;
            };
            send_event(
                event_tx,
                Sdl3InputEvent {
                    device: opened.descriptor,
                    observed_at_ms: now_ms(),
                    kind: Sdl3InputEventKind::Disconnected,
                },
            )
        }
        Event::ControllerDeviceRemapped { which, .. } => {
            let device_id = joystick_instance_id_to_device_id(which);
            let Some(opened) =
                refresh_opened_gamepad(gamepad_subsystem, opened_gamepads, which, &device_id)
            else {
                return true;
            };
            send_event(
                event_tx,
                Sdl3InputEvent {
                    device: opened.descriptor.clone(),
                    observed_at_ms: now_ms(),
                    kind: Sdl3InputEventKind::Connected,
                },
            )
        }
        Event::ControllerAxisMotion {
            which,
            axis,
            value,
            timestamp,
        } => {
            let device_id = joystick_instance_id_to_device_id(which);
            let Some(opened) =
                refresh_opened_gamepad(gamepad_subsystem, opened_gamepads, which, &device_id)
            else {
                return true;
            };
            let observed_at_ms = timestamp;
            let descriptor = opened.descriptor.clone();
            let events = translate_axis_event(descriptor, observed_at_ms, axis, value);
            for next in events {
                if !send_event(event_tx, next) {
                    return false;
                }
            }
            true
        }
        Event::ControllerButtonDown {
            which,
            button,
            timestamp,
        } => {
            let device_id = joystick_instance_id_to_device_id(which);
            let Some(opened) =
                refresh_opened_gamepad(gamepad_subsystem, opened_gamepads, which, &device_id)
            else {
                return true;
            };
            translate_button_event(opened.descriptor.clone(), timestamp, button, 1.0)
                .map(|event| send_event(event_tx, event))
                .unwrap_or(true)
        }
        Event::ControllerButtonUp {
            which,
            button,
            timestamp,
        } => {
            let device_id = joystick_instance_id_to_device_id(which);
            let Some(opened) =
                refresh_opened_gamepad(gamepad_subsystem, opened_gamepads, which, &device_id)
            else {
                return true;
            };
            translate_button_event(opened.descriptor.clone(), timestamp, button, 0.0)
                .map(|event| send_event(event_tx, event))
                .unwrap_or(true)
        }
        _ => true,
    }
}

fn refresh_opened_gamepad<'a>(
    gamepad_subsystem: &sdl3::GamepadSubsystem,
    opened_gamepads: &'a mut HashMap<String, OpenedSdl3Gamepad>,
    joystick_instance_id: u32,
    device_id: &str,
) -> Option<&'a mut OpenedSdl3Gamepad> {
    if !opened_gamepads.contains_key(device_id) {
        let (resolved_device_id, opened) =
            open_gamepad_from_event(gamepad_subsystem, joystick_instance_id)?;
        opened_gamepads.insert(resolved_device_id.clone(), opened);
        let opened = opened_gamepads.get_mut(&resolved_device_id)?;
        opened.descriptor = descriptor_from_gamepad(resolved_device_id, &opened.gamepad);
        return Some(opened);
    }

    let opened = opened_gamepads.get_mut(device_id)?;
    opened.descriptor = descriptor_from_gamepad(device_id.to_owned(), &opened.gamepad);
    Some(opened)
}

fn open_gamepad(
    gamepad_subsystem: &sdl3::GamepadSubsystem,
    joystick_id: JoystickId,
) -> Option<(String, OpenedSdl3Gamepad)> {
    let gamepad = gamepad_subsystem.open(joystick_id).ok()?;
    let device_id = gamepad_instance_id_to_device_id(&gamepad)?;
    let descriptor = descriptor_from_gamepad(device_id.clone(), &gamepad);
    Some((
        device_id,
        OpenedSdl3Gamepad {
            gamepad,
            descriptor,
        },
    ))
}

fn open_gamepad_from_event(
    gamepad_subsystem: &sdl3::GamepadSubsystem,
    joystick_instance_id: u32,
) -> Option<(String, OpenedSdl3Gamepad)> {
    open_gamepad(
        gamepad_subsystem,
        sdl3::sys::joystick::SDL_JoystickID(joystick_instance_id),
    )
}

fn drain_rumble_commands(
    command_rx: &Receiver<Sdl3RumbleCommand>,
    opened_gamepads: &mut HashMap<String, OpenedSdl3Gamepad>,
) {
    loop {
        match command_rx.try_recv() {
            Ok(Sdl3RumbleCommand::Play { device_ids, effect }) => {
                play_rumble_on_devices(opened_gamepads, &device_ids, &effect);
            }
            Ok(Sdl3RumbleCommand::Stop { device_ids }) => {
                stop_rumble_on_devices(opened_gamepads, &device_ids);
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
        }
    }
}

fn play_rumble_on_devices(
    opened_gamepads: &mut HashMap<String, OpenedSdl3Gamepad>,
    device_ids: &[String],
    effect: &OhMyGamepadRumbleEffectDto,
) {
    let low = normalize_rumble_magnitude(effect.strong_magnitude.max(effect.left_trigger));
    let high = normalize_rumble_magnitude(effect.weak_magnitude.max(effect.right_trigger));
    let left_trigger = normalize_rumble_magnitude(effect.left_trigger);
    let right_trigger = normalize_rumble_magnitude(effect.right_trigger);
    let duration_ms = u32::from(effect.duration_ms.max(16));

    for device_id in device_ids {
        let Some(opened) = opened_gamepads.get_mut(device_id) else {
            continue;
        };
        let _ = opened.gamepad.set_rumble(low, high, duration_ms);
        let _ = opened
            .gamepad
            .set_rumble_triggers(left_trigger, right_trigger, duration_ms);
    }
}

fn stop_rumble_on_devices(
    opened_gamepads: &mut HashMap<String, OpenedSdl3Gamepad>,
    device_ids: &[String],
) {
    for device_id in device_ids {
        let Some(opened) = opened_gamepads.get_mut(device_id) else {
            continue;
        };
        let _ = opened.gamepad.set_rumble(0, 0, 0);
        let _ = opened.gamepad.set_rumble_triggers(0, 0, 0);
    }
}

fn translate_button_event(
    descriptor: Sdl3DeviceDescriptor,
    observed_at_ms: u64,
    button: Button,
    value: f32,
) -> Option<Sdl3InputEvent> {
    let index = match button {
        Button::South => 0,
        Button::East => 1,
        Button::West => 2,
        Button::North => 3,
        Button::LeftShoulder => 4,
        Button::RightShoulder => 5,
        Button::Back => 8,
        Button::Start => 9,
        Button::LeftStick => 10,
        Button::RightStick => 11,
        Button::DPadUp => 12,
        Button::DPadDown => 13,
        Button::DPadLeft => 14,
        Button::DPadRight => 15,
        Button::Guide => 16,
        _ => return None,
    };

    Some(Sdl3InputEvent {
        device: descriptor,
        observed_at_ms,
        kind: Sdl3InputEventKind::ButtonChanged { index, value },
    })
}

fn translate_axis_event(
    descriptor: Sdl3DeviceDescriptor,
    observed_at_ms: u64,
    axis: Axis,
    value: i16,
) -> Vec<Sdl3InputEvent> {
    match axis {
        Axis::LeftX => vec![axis_event(
            descriptor,
            observed_at_ms,
            0,
            normalize_stick_axis(value),
        )],
        Axis::LeftY => vec![axis_event(
            descriptor,
            observed_at_ms,
            1,
            normalize_stick_axis(value),
        )],
        Axis::RightX => vec![axis_event(
            descriptor,
            observed_at_ms,
            2,
            normalize_stick_axis(value),
        )],
        Axis::RightY => vec![axis_event(
            descriptor,
            observed_at_ms,
            3,
            normalize_stick_axis(value),
        )],
        Axis::TriggerLeft => {
            let axis_value = normalize_trigger_axis(value);
            let button_value = axis_to_trigger_button_value(axis_value);
            vec![
                button_event(descriptor.clone(), observed_at_ms, 6, button_value),
                axis_event(descriptor, observed_at_ms, 4, axis_value),
            ]
        }
        Axis::TriggerRight => {
            let axis_value = normalize_trigger_axis(value);
            let button_value = axis_to_trigger_button_value(axis_value);
            vec![
                button_event(descriptor.clone(), observed_at_ms, 7, button_value),
                axis_event(descriptor, observed_at_ms, 5, axis_value),
            ]
        }
    }
}

fn button_event(
    device: Sdl3DeviceDescriptor,
    observed_at_ms: u64,
    index: usize,
    value: f32,
) -> Sdl3InputEvent {
    Sdl3InputEvent {
        device,
        observed_at_ms,
        kind: Sdl3InputEventKind::ButtonChanged { index, value },
    }
}

fn axis_event(
    device: Sdl3DeviceDescriptor,
    observed_at_ms: u64,
    index: usize,
    value: f32,
) -> Sdl3InputEvent {
    Sdl3InputEvent {
        device,
        observed_at_ms,
        kind: Sdl3InputEventKind::AxisChanged { index, value },
    }
}

fn descriptor_from_gamepad(device_id: String, gamepad: &Gamepad) -> Sdl3DeviceDescriptor {
    let power_info = gamepad.power_info();
    let basic_rumble = unsafe { gamepad.has_rumble() };
    let trigger_rumble = unsafe { gamepad.has_rumble_triggers() };
    let has_led = unsafe { gamepad.has_led() };
    let touchpad_count = gamepad.touchpads_count();
    let touchpad_finger_count = (touchpad_count > 0)
        .then(|| gamepad.supported_touchpad_fingers(0))
        .filter(|count| *count > 0);
    let mapping = gamepad.mapping();
    let mapping_supported = mapping.is_some();
    let player_index = gamepad.player_index();
    let serial_number = gamepad.serial_number();
    let serial_supported = serial_number.is_some();

    Sdl3DeviceDescriptor {
        device_id,
        name: gamepad.name().unwrap_or_else(|| "Controller".to_owned()),
        connection: connection_from_state(gamepad.connection_state().ok()),
        vendor_id: gamepad.vendor_id(),
        product_id: gamepad.product_id(),
        product_version: gamepad.product_version(),
        firmware_version: gamepad.firmware_version(),
        serial_number,
        path: gamepad.path(),
        mapping,
        player_index,
        gamepad_type: Some(device_type_from_sdl(gamepad.r#type())),
        power_state: Some(power_state_from_info(&power_info)),
        battery_percent: battery_percent(&power_info),
        touchpad_count: (touchpad_count > 0).then_some(touchpad_count),
        touchpad_finger_count,
        capabilities: OhMyGamepadCapabilityFlagsDto {
            supports_rumble: basic_rumble,
            supports_trigger_rumble: trigger_rumble,
            reports_battery: has_battery(&power_info),
            supports_player_index: player_index.is_some(),
            reports_mapping: mapping_supported,
            supports_touchpad: touchpad_count > 0,
            supports_led: has_led,
            reports_serial: serial_supported,
        },
    }
}

fn connection_from_state(
    connection_state: Option<ConnectionState>,
) -> Option<OhMyGamepadConnectionKindDto> {
    match connection_state {
        Some(ConnectionState::Wired) => Some(OhMyGamepadConnectionKindDto::Usb),
        Some(ConnectionState::Wireless) => Some(OhMyGamepadConnectionKindDto::Unknown),
        Some(ConnectionState::Unknown | ConnectionState::Invalid) | None => None,
    }
}

fn has_battery(power_info: &PowerInfo) -> bool {
    matches!(
        power_info.state,
        PowerLevel::OnBattery | PowerLevel::Charging | PowerLevel::Charged
    )
}

fn battery_percent(power_info: &PowerInfo) -> Option<u8> {
    let percentage = power_info.percentage;
    (0..=100).contains(&percentage).then_some(percentage as u8)
}

fn power_state_from_info(power_info: &PowerInfo) -> OhMyGamepadPowerStateDto {
    match power_info.state {
        PowerLevel::Unknown | PowerLevel::Error => OhMyGamepadPowerStateDto::Unknown,
        PowerLevel::NoBattery => OhMyGamepadPowerStateDto::Wired,
        PowerLevel::OnBattery => OhMyGamepadPowerStateDto::OnBattery,
        PowerLevel::Charging => OhMyGamepadPowerStateDto::Charging,
        PowerLevel::Charged => OhMyGamepadPowerStateDto::Charged,
    }
}

fn device_type_from_sdl(value: GamepadType) -> OhMyGamepadDeviceTypeDto {
    match value {
        GamepadType::Unknown => OhMyGamepadDeviceTypeDto::Unknown,
        GamepadType::Standard => OhMyGamepadDeviceTypeDto::Standard,
        GamepadType::Xbox360 => OhMyGamepadDeviceTypeDto::Xbox360,
        GamepadType::XboxOne => OhMyGamepadDeviceTypeDto::XboxOne,
        GamepadType::PS3 => OhMyGamepadDeviceTypeDto::Ps3,
        GamepadType::PS4 => OhMyGamepadDeviceTypeDto::Ps4,
        GamepadType::PS5 => OhMyGamepadDeviceTypeDto::Ps5,
        GamepadType::NintendoSwitchPro => OhMyGamepadDeviceTypeDto::NintendoSwitchPro,
        GamepadType::NintendoSwitchJoyconLeft => OhMyGamepadDeviceTypeDto::NintendoSwitchJoyconLeft,
        GamepadType::NintendoSwitchJoyconRight => {
            OhMyGamepadDeviceTypeDto::NintendoSwitchJoyconRight
        }
        GamepadType::NintendoSwitchJoyconPair => OhMyGamepadDeviceTypeDto::NintendoSwitchJoyconPair,
    }
}

fn normalize_rumble_magnitude(value: f32) -> u16 {
    (value.clamp(0.0, 1.0) * u16::MAX as f32).round() as u16
}

fn normalize_stick_axis(value: i16) -> f32 {
    (value as f32 / i16::MAX as f32).clamp(-1.0, 1.0)
}

fn normalize_trigger_axis(value: i16) -> f32 {
    let normalized = (value.max(0) as f32 / i16::MAX as f32).clamp(0.0, 1.0);
    (normalized * 2.0) - 1.0
}

fn axis_to_trigger_button_value(axis_value: f32) -> f32 {
    ((axis_value + 1.0) * 0.5).clamp(0.0, 1.0)
}

fn gamepad_instance_id_to_device_id(gamepad: &Gamepad) -> Option<String> {
    Some(gamepad.id().ok()?.0.to_string())
}

fn joystick_instance_id_to_device_id(joystick_instance_id: u32) -> String {
    joystick_instance_id.to_string()
}

fn send_event(event_tx: &Sender<Sdl3InputEvent>, event: Sdl3InputEvent) -> bool {
    event_tx.send(event).is_ok()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
