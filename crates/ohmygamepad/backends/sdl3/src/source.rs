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
    hint,
    joystick::{ConnectionState, JoystickId, PowerInfo, PowerLevel},
    sensor::SensorType,
};

use crate::{Sdl3BackendConfig, Sdl3DeviceDescriptor, Sdl3InputEvent, Sdl3InputEventKind};

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

    pub fn prime_sampling(&self) -> Result<(), Sdl3RumbleError> {
        let (ack_tx, ack_rx) = mpsc::channel();
        self.command_tx
            .send(Sdl3RumbleCommand::PrimeSampling { ack_tx })
            .map_err(|_| Sdl3RumbleError::CommandChannelClosed)?;
        ack_rx
            .recv_timeout(Duration::from_millis(100))
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
    PrimeSampling {
        ack_tx: Sender<()>,
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
    pub fn new(
        config: Sdl3BackendConfig,
    ) -> Result<(Self, Option<Sdl3RumbleHandle>), Sdl3SourceInitError> {
        let (event_tx, event_rx) = mpsc::channel();
        let (command_tx, command_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();

        thread::Builder::new()
            .name("ohmygamepad-sdl3-source".to_owned())
            .spawn(move || run_sdl3_source_thread(config, event_tx, command_rx, ready_tx))
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
    config: Sdl3BackendConfig,
    event_tx: Sender<Sdl3InputEvent>,
    command_rx: Receiver<Sdl3RumbleCommand>,
    ready_tx: Sender<Result<(), Sdl3SourceInitError>>,
) {
    let _ = hint::set("SDL_JOYSTICK_ALLOW_BACKGROUND_EVENTS", "1");

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
    let joystick_subsystem = match sdl.joystick() {
        Ok(value) => value,
        Err(error) => {
            let _ = ready_tx.send(Err(Sdl3SourceInitError::new(format!(
                "init sdl3 joystick subsystem failed: {error}"
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

    apply_gamepad_mappings(&gamepad_subsystem, &config);
    log_unmapped_joysticks(&joystick_subsystem, &gamepad_subsystem);
    gamepad_subsystem.set_events_processing_state(true);

    let mut opened_gamepads = HashMap::new();
    if let Ok(gamepad_ids) = gamepad_subsystem.gamepads() {
        for gamepad_id in gamepad_ids {
            if let Some((device_id, opened)) = open_gamepad(&gamepad_subsystem, gamepad_id, &config)
            {
                if dedupe_or_insert_opened_gamepad(
                    &mut opened_gamepads,
                    &opened,
                    DuplicatePolicy::InitialEnumerate,
                ) {
                    continue;
                }
                let observed_at_ms = now_ms();
                log_gamepad_diagnostics("initial-enumerate", &opened.descriptor);
                if !emit_connected_baseline_event(
                    &event_tx,
                    &opened.descriptor,
                    &opened.gamepad,
                    observed_at_ms,
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
        drain_rumble_commands(&event_tx, &command_rx, &mut opened_gamepads);
        gamepad_subsystem.update();

        let mut received_event = false;
        while let Some(event) = event_pump.poll_event() {
            received_event = true;
            if !handle_sdl_event(
                &event_tx,
                &gamepad_subsystem,
                &mut opened_gamepads,
                &config,
                event,
            ) {
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
    config: &Sdl3BackendConfig,
    event: Event,
) -> bool {
    match event {
        Event::ControllerDeviceAdded { which, .. } => {
            if let Some((device_id, opened)) =
                open_gamepad_from_event(gamepad_subsystem, which, config)
            {
                if dedupe_or_insert_opened_gamepad(
                    opened_gamepads,
                    &opened,
                    DuplicatePolicy::HotplugAdd,
                ) {
                    return true;
                }
                log_gamepad_diagnostics("connected", &opened.descriptor);
                let observed_at_ms = now_ms();
                let lookup_device_id = device_id.clone();
                opened_gamepads.insert(device_id, opened);
                let Some(opened) = opened_gamepads.get(&lookup_device_id) else {
                    return true;
                };
                emit_connected_baseline_event(
                    event_tx,
                    &opened.descriptor,
                    &opened.gamepad,
                    observed_at_ms,
                )
            } else {
                true
            }
        }
        Event::ControllerDeviceRemoved { which, .. } => {
            let device_id = joystick_instance_id_to_device_id(which);
            let Some(opened) = opened_gamepads.remove(&device_id) else {
                log::info!(
                    "sdl3_gamepad_remove_ignored reason=missing-instance device_id={}",
                    device_id
                );
                return true;
            };
            log_gamepad_diagnostics("disconnected", &opened.descriptor);
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
            let Some(opened) = refresh_opened_gamepad(
                gamepad_subsystem,
                opened_gamepads,
                which,
                &device_id,
                config,
            ) else {
                return true;
            };
            log_gamepad_diagnostics("remapped", &opened.descriptor);
            emit_connected_baseline_event(event_tx, &opened.descriptor, &opened.gamepad, now_ms())
        }
        Event::ControllerAxisMotion {
            which,
            axis,
            value,
            timestamp,
        } => {
            let device_id = joystick_instance_id_to_device_id(which);
            let Some(opened) = refresh_opened_gamepad(
                gamepad_subsystem,
                opened_gamepads,
                which,
                &device_id,
                config,
            ) else {
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
            let Some(opened) = refresh_opened_gamepad(
                gamepad_subsystem,
                opened_gamepads,
                which,
                &device_id,
                config,
            ) else {
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
            let Some(opened) = refresh_opened_gamepad(
                gamepad_subsystem,
                opened_gamepads,
                which,
                &device_id,
                config,
            ) else {
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
    config: &Sdl3BackendConfig,
) -> Option<&'a mut OpenedSdl3Gamepad> {
    if !opened_gamepads.contains_key(device_id) {
        log::info!(
            "sdl3_gamepad_refresh_reopen reason=missing-tracked-device event_device_id={} instance_id={}",
            device_id,
            joystick_instance_id
        );
        let (resolved_device_id, opened) =
            open_gamepad_from_event(gamepad_subsystem, joystick_instance_id, config)?;
        if dedupe_or_insert_opened_gamepad(opened_gamepads, &opened, DuplicatePolicy::RemapRefresh)
        {
            return None;
        }
        opened_gamepads.insert(resolved_device_id.clone(), opened);
        let opened = opened_gamepads.get_mut(&resolved_device_id)?;
        opened.descriptor = descriptor_from_gamepad(resolved_device_id, &opened.gamepad);
        log_gamepad_diagnostics("refresh-reopen", &opened.descriptor);
        return Some(opened);
    }

    let opened = opened_gamepads.get_mut(device_id)?;
    let refreshed = descriptor_from_gamepad(device_id.to_owned(), &opened.gamepad);
    if opened.descriptor != refreshed {
        let previous_key = duplicate_fingerprint(&opened.descriptor);
        let next_key = duplicate_fingerprint(&refreshed);
        opened.descriptor = refreshed;
        log::info!(
            "sdl3_gamepad_refresh_update device_id={} duplicate_key={} previous_key={} mapping_guid_hint={} player_index={} path={} serial={}",
            opened.descriptor.device_id,
            next_key,
            previous_key,
            opened
                .descriptor
                .mapping
                .as_deref()
                .map(mapping_guid_hint)
                .unwrap_or_default(),
            opened
                .descriptor
                .player_index
                .map(|value| value.to_string())
                .unwrap_or_default(),
            opened.descriptor.path.as_deref().unwrap_or_default(),
            opened
                .descriptor
                .serial_number
                .as_deref()
                .unwrap_or_default(),
        );
    } else {
        opened.descriptor = refreshed;
    }
    Some(opened)
}

fn open_gamepad(
    gamepad_subsystem: &sdl3::GamepadSubsystem,
    joystick_id: JoystickId,
    config: &Sdl3BackendConfig,
) -> Option<(String, OpenedSdl3Gamepad)> {
    let gamepad = gamepad_subsystem.open(joystick_id).ok()?;
    if should_ignore_gamepad(&gamepad, config) {
        return None;
    }
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
    config: &Sdl3BackendConfig,
) -> Option<(String, OpenedSdl3Gamepad)> {
    open_gamepad(
        gamepad_subsystem,
        sdl3::sys::joystick::SDL_JoystickID(joystick_instance_id),
        config,
    )
}

#[derive(Copy, Clone, Debug)]
enum DuplicatePolicy {
    InitialEnumerate,
    HotplugAdd,
    RemapRefresh,
}

fn apply_gamepad_mappings(gamepad_subsystem: &sdl3::GamepadSubsystem, config: &Sdl3BackendConfig) {
    for path in &config.mapping_paths {
        match gamepad_subsystem.load_mappings(path) {
            Ok(count) => {
                if count > 0 {
                    log::info!(
                        "loaded {} SDL gamepad mappings from {}",
                        count,
                        path.display()
                    );
                } else {
                    log::warn!("loaded 0 SDL gamepad mappings from {}", path.display());
                }
            }
            Err(error) => {
                log::warn!(
                    "failed to load SDL gamepad mappings from {}: {:?}",
                    path.display(),
                    error
                );
            }
        }
    }

    for mapping in &config.extra_mappings {
        match gamepad_subsystem.add_mapping(mapping) {
            Ok(status) => {
                log::info!("applied SDL gamepad mapping overlay: {:?}", status);
            }
            Err(error) => {
                log::warn!("failed to apply SDL gamepad mapping overlay: {:?}", error);
            }
        }
    }
}

fn should_ignore_gamepad(gamepad: &Gamepad, config: &Sdl3BackendConfig) -> bool {
    let guid = guid_for_gamepad(gamepad).map(|value| value.to_ascii_lowercase());
    if let Some(guid) = guid.as_deref() {
        if config
            .ignored_device_guids
            .iter()
            .any(|item| item.eq_ignore_ascii_case(guid))
        {
            log_ignored_gamepad("guid", gamepad, Some(guid));
            return true;
        }
    }

    let vendor_id = gamepad.vendor_id();
    let product_id = gamepad.product_id();
    if let (Some(vendor_id), Some(product_id)) = (vendor_id, product_id) {
        if config
            .ignored_vid_pids
            .iter()
            .any(|(vendor, product)| *vendor == vendor_id && *product == product_id)
        {
            log_ignored_gamepad(
                "vid-pid",
                gamepad,
                Some(&format!("{vendor_id:04x}:{product_id:04x}")),
            );
            return true;
        }
    }

    let name = gamepad.name().unwrap_or_default();
    if config
        .ignored_name_contains
        .iter()
        .any(|pattern| !pattern.is_empty() && name.contains(pattern))
    {
        log_ignored_gamepad("name-match", gamepad, Some(name.as_str()));
        return true;
    }

    false
}

fn dedupe_or_insert_opened_gamepad(
    opened_gamepads: &mut HashMap<String, OpenedSdl3Gamepad>,
    candidate: &OpenedSdl3Gamepad,
    policy: DuplicatePolicy,
) -> bool {
    let candidate_key = duplicate_fingerprint(&candidate.descriptor);
    let candidate_id = &candidate.descriptor.device_id;

    if opened_gamepads.contains_key(candidate_id) {
        return false;
    }

    let duplicate = opened_gamepads
        .iter()
        .find(|(_, opened)| duplicate_fingerprint(&opened.descriptor) == candidate_key);

    let Some((existing_id, existing)) = duplicate else {
        return false;
    };

    log::warn!(
        "sdl3_duplicate_gamepad policy={:?} incoming={} existing={} key={} incoming_path={} existing_path={} incoming_serial={} existing_serial={}",
        policy,
        candidate.descriptor.device_id,
        existing_id,
        duplicate_fingerprint(&existing.descriptor),
        candidate.descriptor.path.as_deref().unwrap_or_default(),
        existing.descriptor.path.as_deref().unwrap_or_default(),
        candidate
            .descriptor
            .serial_number
            .as_deref()
            .unwrap_or_default(),
        existing
            .descriptor
            .serial_number
            .as_deref()
            .unwrap_or_default()
    );
    true
}

fn emit_connected_baseline_event(
    event_tx: &Sender<Sdl3InputEvent>,
    descriptor: &Sdl3DeviceDescriptor,
    gamepad: &Gamepad,
    observed_at_ms: u64,
) -> bool {
    if !send_event(
        event_tx,
        Sdl3InputEvent {
            device: descriptor.clone(),
            observed_at_ms,
            kind: Sdl3InputEventKind::Connected,
        },
    ) {
        return false;
    }

    let (buttons, axes) = capture_gamepad_baseline_state(gamepad);
    send_event(
        event_tx,
        Sdl3InputEvent {
            device: descriptor.clone(),
            observed_at_ms,
            kind: Sdl3InputEventKind::Snapshot { buttons, axes },
        },
    )
}

fn duplicate_fingerprint(descriptor: &Sdl3DeviceDescriptor) -> String {
    if let Some(path) = descriptor.path.as_deref().filter(|value| !value.is_empty()) {
        return format!("path:{path}");
    }
    if let Some(serial) = descriptor
        .serial_number
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        return format!(
            "serial:{:04x}:{:04x}:{serial}",
            descriptor.vendor_id.unwrap_or_default(),
            descriptor.product_id.unwrap_or_default()
        );
    }
    format!(
        "fallback:{:04x}:{:04x}:{}:{}",
        descriptor.vendor_id.unwrap_or_default(),
        descriptor.product_id.unwrap_or_default(),
        descriptor.product_version.unwrap_or_default(),
        descriptor.name.trim()
    )
}

fn guid_for_gamepad(gamepad: &Gamepad) -> Option<String> {
    let joystick_id = gamepad.id().ok()?;
    let guid = gamepad.subsystem().guid_for_id(joystick_id);
    (!guid.is_zero()).then(|| guid.string())
}

fn log_unmapped_joysticks(
    joystick_subsystem: &sdl3::JoystickSubsystem,
    gamepad_subsystem: &sdl3::GamepadSubsystem,
) {
    let Ok(joystick_ids) = joystick_subsystem.joysticks() else {
        return;
    };

    for joystick_id in joystick_ids {
        if gamepad_subsystem.is_gamepad(joystick_id) {
            continue;
        }
        let Ok(joystick) = joystick_subsystem.open(joystick_id) else {
            continue;
        };
        let guid = joystick.guid().string();
        log::warn!(
            "sdl3_unmapped_joystick name=\"{}\" guid={} instance_id={}",
            joystick.name(),
            guid,
            joystick.id()
        );
    }
}

fn log_ignored_gamepad(reason: &str, gamepad: &Gamepad, match_value: Option<&str>) {
    log::info!(
        "sdl3_ignored_gamepad reason={} match={} name=\"{}\" guid={} vid={:04x} pid={:04x} path={} mapping_present={}",
        reason,
        match_value.unwrap_or(""),
        gamepad.name().unwrap_or_default(),
        guid_for_gamepad(gamepad).unwrap_or_default(),
        gamepad.vendor_id().unwrap_or_default(),
        gamepad.product_id().unwrap_or_default(),
        gamepad.path().unwrap_or_default(),
        gamepad.mapping().is_some(),
    );
}

fn log_gamepad_diagnostics(stage: &str, descriptor: &Sdl3DeviceDescriptor) {
    log::info!(
        "sdl3_gamepad_diagnostics stage={} device_id={} name=\"{}\" type={:?} connection={:?} guid_hint={} vid={:04x} pid={:04x} product_ver={:04x} firmware_ver={:04x} path={} serial={} mapped={} player_index={} power={:?} battery_percent={} touchpads={} fingers={} duplicate_key={} caps=rumble:{} trigger_rumble:{} battery:{} mapping:{} touchpad:{} accel:{} gyro:{} led:{} serial:{}",
        stage,
        descriptor.device_id,
        descriptor.name,
        descriptor.gamepad_type,
        descriptor.connection,
        descriptor
            .mapping
            .as_deref()
            .map(mapping_guid_hint)
            .unwrap_or_default(),
        descriptor.vendor_id.unwrap_or_default(),
        descriptor.product_id.unwrap_or_default(),
        descriptor.product_version.unwrap_or_default(),
        descriptor.firmware_version.unwrap_or_default(),
        descriptor.path.as_deref().unwrap_or_default(),
        descriptor.serial_number.as_deref().unwrap_or_default(),
        descriptor.mapping.as_ref().map(|value| !value.is_empty()).unwrap_or(false),
        descriptor.player_index.map(|value| value.to_string()).unwrap_or_default(),
        descriptor
            .power_state
            .map(|value| format!("{value:?}"))
            .unwrap_or_default(),
        descriptor
            .battery_percent
            .map(|value| value.to_string())
            .unwrap_or_default(),
        descriptor.touchpad_count.map(|value| value.to_string()).unwrap_or_default(),
        descriptor
            .touchpad_finger_count
            .map(|value| value.to_string())
            .unwrap_or_default(),
        duplicate_fingerprint(descriptor),
        descriptor.capabilities.supports_rumble,
        descriptor.capabilities.supports_trigger_rumble,
        descriptor.capabilities.reports_battery,
        descriptor.capabilities.reports_mapping,
        descriptor.capabilities.supports_touchpad,
        descriptor.capabilities.supports_accel,
        descriptor.capabilities.supports_gyro,
        descriptor.capabilities.supports_led,
        descriptor.capabilities.reports_serial,
    );
}

fn mapping_guid_hint(mapping: &str) -> &str {
    mapping.split(',').next().unwrap_or_default()
}

fn drain_rumble_commands(
    event_tx: &Sender<Sdl3InputEvent>,
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
            Ok(Sdl3RumbleCommand::PrimeSampling { ack_tx }) => {
                prime_sampling_on_devices(event_tx, opened_gamepads);
                let _ = ack_tx.send(());
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

fn prime_sampling_on_devices(
    event_tx: &Sender<Sdl3InputEvent>,
    opened_gamepads: &mut HashMap<String, OpenedSdl3Gamepad>,
) {
    let observed_at_ms = now_ms();
    let mut device_ids = opened_gamepads.keys().cloned().collect::<Vec<_>>();
    device_ids.sort();
    for device_id in device_ids {
        let Some(opened) = opened_gamepads.get_mut(&device_id) else {
            continue;
        };
        opened.descriptor = descriptor_from_gamepad(device_id, &opened.gamepad);
        log_gamepad_diagnostics("prime-snapshot", &opened.descriptor);
        let (buttons, axes) = capture_gamepad_baseline_state(&opened.gamepad);
        let _ = send_event(
            event_tx,
            Sdl3InputEvent {
                device: opened.descriptor.clone(),
                observed_at_ms,
                kind: Sdl3InputEventKind::Snapshot { buttons, axes },
            },
        );
    }
}

fn capture_gamepad_baseline_state(gamepad: &Gamepad) -> (Vec<f32>, Vec<f32>) {
    let mut buttons = vec![0.0; 17];
    let mut axes = vec![0.0; 6];

    for (index, pressed) in [
        (0, gamepad.button(Button::South)),
        (1, gamepad.button(Button::East)),
        (2, gamepad.button(Button::West)),
        (3, gamepad.button(Button::North)),
        (4, gamepad.button(Button::LeftShoulder)),
        (5, gamepad.button(Button::RightShoulder)),
        (8, gamepad.button(Button::Back)),
        (9, gamepad.button(Button::Start)),
        (10, gamepad.button(Button::LeftStick)),
        (11, gamepad.button(Button::RightStick)),
        (12, gamepad.button(Button::DPadUp)),
        (13, gamepad.button(Button::DPadDown)),
        (14, gamepad.button(Button::DPadLeft)),
        (15, gamepad.button(Button::DPadRight)),
        (
            16,
            gamepad.button(Button::Guide) || gamepad.button(Button::Misc1),
        ),
    ] {
        buttons[index] = bool_to_button_value(pressed);
    }

    axes[0] = normalize_stick_axis(gamepad.axis(Axis::LeftX));
    axes[1] = normalize_stick_axis(gamepad.axis(Axis::LeftY));
    axes[2] = normalize_stick_axis(gamepad.axis(Axis::RightX));
    axes[3] = -normalize_stick_axis(gamepad.axis(Axis::RightY));

    let left_trigger = normalize_trigger_axis(gamepad.axis(Axis::TriggerLeft));
    let right_trigger = normalize_trigger_axis(gamepad.axis(Axis::TriggerRight));
    axes[4] = left_trigger;
    axes[5] = right_trigger;
    buttons[6] = axis_to_trigger_button_value(left_trigger);
    buttons[7] = axis_to_trigger_button_value(right_trigger);

    (buttons, axes)
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
        // 某些驱动/手柄会把中间系统键上报为 Misc1，这里统一映射到 home。
        Button::Guide | Button::Misc1 => 16,
        // 标准布局之外的按钮走开放索引，避免采样层把按键直接丢掉。
        _ => fallback_button_index(button)?,
    };

    Some(Sdl3InputEvent {
        device: descriptor,
        observed_at_ms,
        kind: Sdl3InputEventKind::ButtonChanged { index, value },
    })
}

fn fallback_button_index(button: Button) -> Option<usize> {
    let raw = button as i32;
    if raw < 0 {
        return None;
    }
    // 1000+ 预留给 SDL 扩展按钮，避免和标准逻辑布局索引冲突。
    Some(1000 + raw as usize)
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
            -normalize_stick_axis(value),
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
    let has_accel = unsafe { gamepad.has_sensor(SensorType::Accelerometer) };
    let has_gyro = unsafe { gamepad.has_sensor(SensorType::Gyroscope) };
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
            supports_accel: has_accel,
            supports_gyro: has_gyro,
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

fn bool_to_button_value(pressed: bool) -> f32 {
    if pressed {
        1.0
    } else {
        0.0
    }
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
