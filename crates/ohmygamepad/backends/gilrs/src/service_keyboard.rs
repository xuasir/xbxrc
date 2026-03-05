use std::{
    collections::HashSet,
    panic::AssertUnwindSafe,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread,
};

use ohmygamepad_protocol::{LogicalPadStateDto, OhMyGamepadKeyboardMappingDto};

use crate::{
    GilrsDeviceDescriptor, GilrsInputEvent, GilrsInputEventKind,
    OhMyGamepadDesktopKeyboardListener, OhMyGamepadDesktopKeyboardListenerConfig,
};

pub(crate) const KEYBOARD_FALLBACK_DEVICE_ID: &str = "virtual:keyboard";
pub(crate) const KEYBOARD_FALLBACK_DEVICE_NAME: &str = "Keyboard Fallback";

#[derive(Debug)]
pub(crate) struct ServiceKeyboardFallbackGate {
    enabled: bool,
    connected: bool,
    active_non_keyboard_device_ids: HashSet<String>,
}

impl ServiceKeyboardFallbackGate {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            connected: enabled,
            active_non_keyboard_device_ids: HashSet::new(),
        }
    }

    pub(crate) fn initial_events(&self, observed_at_ms: u64) -> Vec<GilrsInputEvent> {
        if self.enabled {
            vec![keyboard_connection_event(true, observed_at_ms)]
        } else {
            Vec::new()
        }
    }

    pub(crate) fn transform_event(
        &mut self,
        event: GilrsInputEvent,
        observed_at_ms: u64,
    ) -> Vec<GilrsInputEvent> {
        let is_keyboard = event.device.device_id == KEYBOARD_FALLBACK_DEVICE_ID;
        if is_keyboard && !self.connected {
            return Vec::new();
        }

        let had_non_keyboard_devices = !self.active_non_keyboard_device_ids.is_empty();
        if !is_keyboard {
            match event.kind {
                GilrsInputEventKind::Connected => {
                    self.active_non_keyboard_device_ids
                        .insert(event.device.device_id.clone());
                }
                GilrsInputEventKind::Disconnected => {
                    self.active_non_keyboard_device_ids
                        .remove(&event.device.device_id);
                }
                _ => {}
            }
        }
        let has_non_keyboard_devices = !self.active_non_keyboard_device_ids.is_empty();

        let mut result = Vec::new();
        if self.enabled
            && !is_keyboard
            && !had_non_keyboard_devices
            && has_non_keyboard_devices
            && self.connected
        {
            self.connected = false;
            result.push(keyboard_connection_event(false, observed_at_ms));
        }

        result.push(event);

        if self.enabled
            && !is_keyboard
            && had_non_keyboard_devices
            && !has_non_keyboard_devices
            && !self.connected
        {
            self.connected = true;
            result.push(keyboard_connection_event(true, observed_at_ms));
        }

        result
    }
}

enum KeyboardListenerCommand {
    ReplaceMapping(OhMyGamepadKeyboardMappingDto),
    Shutdown,
}

pub(crate) struct ServiceKeyboardListenerHandle {
    command_tx: Sender<KeyboardListenerCommand>,
    join_handle: thread::JoinHandle<()>,
}

impl ServiceKeyboardListenerHandle {
    pub(crate) fn replace_mapping(&self, mapping: OhMyGamepadKeyboardMappingDto) -> Result<(), ()> {
        self.command_tx
            .send(KeyboardListenerCommand::ReplaceMapping(mapping))
            .map_err(|_| ())
    }

    pub(crate) fn shutdown(self) {
        let _ = self.command_tx.send(KeyboardListenerCommand::Shutdown);
        let _ = self.join_handle.join();
    }
}

pub(crate) fn keyboard_connection_event(connected: bool, observed_at_ms: u64) -> GilrsInputEvent {
    GilrsInputEvent {
        device: keyboard_descriptor(),
        observed_at_ms,
        kind: if connected {
            GilrsInputEventKind::Connected
        } else {
            GilrsInputEventKind::Disconnected
        },
    }
}

pub(crate) fn spawn_keyboard_listener_thread(
    config: OhMyGamepadDesktopKeyboardListenerConfig,
    virtual_input_tx: Sender<Vec<GilrsInputEvent>>,
    now_ms: fn() -> u64,
) -> ServiceKeyboardListenerHandle {
    let (command_tx, command_rx) = mpsc::channel();
    let join_handle = thread::spawn(move || {
        let Ok(mut listener) = OhMyGamepadDesktopKeyboardListener::try_new(config) else {
            // 桌面键盘监听不是核心路径；权限缺失时直接降级为“无键盘 fallback”。
            return;
        };
        let mut last_state = LogicalPadStateDto::default();

        loop {
            match drain_keyboard_listener_commands(
                &mut listener,
                &command_rx,
                &virtual_input_tx,
                now_ms,
            ) {
                KeyboardThreadControl::Continue => {}
                KeyboardThreadControl::Shutdown => break,
            }

            let Ok(state) = std::panic::catch_unwind(AssertUnwindSafe(|| listener.poll_state()))
            else {
                break;
            };
            if state != last_state {
                if send_keyboard_state_events(&virtual_input_tx, state.clone(), now_ms).is_err() {
                    break;
                }
                last_state = state;
            }

            thread::sleep(listener.poll_interval());
        }
    });

    ServiceKeyboardListenerHandle {
        command_tx,
        join_handle,
    }
}

enum KeyboardThreadControl {
    Continue,
    Shutdown,
}

fn drain_keyboard_listener_commands(
    listener: &mut OhMyGamepadDesktopKeyboardListener,
    command_rx: &Receiver<KeyboardListenerCommand>,
    virtual_input_tx: &Sender<Vec<GilrsInputEvent>>,
    now_ms: fn() -> u64,
) -> KeyboardThreadControl {
    loop {
        match command_rx.try_recv() {
            Ok(KeyboardListenerCommand::ReplaceMapping(mapping)) => {
                let state = listener.set_mapping(mapping);
                if send_keyboard_state_events(virtual_input_tx, state, now_ms).is_err() {
                    return KeyboardThreadControl::Shutdown;
                }
            }
            Ok(KeyboardListenerCommand::Shutdown) => return KeyboardThreadControl::Shutdown,
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => {
                return KeyboardThreadControl::Continue;
            }
        }
    }
}

fn send_keyboard_state_events(
    tx: &Sender<Vec<GilrsInputEvent>>,
    state: LogicalPadStateDto,
    now_ms: fn() -> u64,
) -> Result<(), ()> {
    tx.send(keyboard_state_events(state, now_ms()))
        .map_err(|_| ())
}

fn keyboard_state_events(state: LogicalPadStateDto, observed_at_ms: u64) -> Vec<GilrsInputEvent> {
    let device = keyboard_descriptor();
    let left_trigger_value = state.buttons.l2.max(state.left_trigger);
    let right_trigger_value = state.buttons.r2.max(state.right_trigger);

    let mut events = Vec::new();
    for (index, value) in [
        (0, state.buttons.south),
        (1, state.buttons.east),
        (2, state.buttons.west),
        (3, state.buttons.north),
        (4, state.buttons.l1),
        (5, state.buttons.r1),
        (6, left_trigger_value),
        (7, right_trigger_value),
        (8, state.buttons.view),
        (9, state.buttons.menu),
        (10, state.buttons.l3),
        (11, state.buttons.r3),
        (12, state.buttons.dpad_up),
        (13, state.buttons.dpad_down),
        (14, state.buttons.dpad_left),
        (15, state.buttons.dpad_right),
        (16, state.buttons.home),
    ] {
        events.push(GilrsInputEvent {
            device: device.clone(),
            observed_at_ms,
            kind: GilrsInputEventKind::ButtonChanged { index, value },
        });
    }

    for (index, value) in [
        (0, state.left_stick.x),
        (1, state.left_stick.y),
        (2, state.right_stick.x),
        (3, state.right_stick.y),
        (4, trigger_axis_value(state.left_trigger)),
        (5, trigger_axis_value(state.right_trigger)),
    ] {
        events.push(GilrsInputEvent {
            device: device.clone(),
            observed_at_ms,
            kind: GilrsInputEventKind::AxisChanged { index, value },
        });
    }

    events
}

fn keyboard_descriptor() -> GilrsDeviceDescriptor {
    GilrsDeviceDescriptor {
        device_id: KEYBOARD_FALLBACK_DEVICE_ID.to_owned(),
        name: KEYBOARD_FALLBACK_DEVICE_NAME.to_owned(),
        ..GilrsDeviceDescriptor::default()
    }
}

fn trigger_axis_value(value: f32) -> f32 {
    (value.clamp(0.0, 1.0) * 2.0) - 1.0
}

#[cfg(test)]
mod tests {
    use super::{
        ServiceKeyboardFallbackGate, KEYBOARD_FALLBACK_DEVICE_ID, KEYBOARD_FALLBACK_DEVICE_NAME,
    };
    use crate::{GilrsDeviceDescriptor, GilrsInputEvent, GilrsInputEventKind};

    fn keyboard_event(kind: GilrsInputEventKind) -> GilrsInputEvent {
        GilrsInputEvent {
            device: GilrsDeviceDescriptor {
                device_id: KEYBOARD_FALLBACK_DEVICE_ID.to_owned(),
                name: KEYBOARD_FALLBACK_DEVICE_NAME.to_owned(),
                ..GilrsDeviceDescriptor::default()
            },
            observed_at_ms: 10,
            kind,
        }
    }

    fn physical_event(device_id: &str, kind: GilrsInputEventKind) -> GilrsInputEvent {
        GilrsInputEvent {
            device: GilrsDeviceDescriptor {
                device_id: device_id.to_owned(),
                name: device_id.to_owned(),
                ..GilrsDeviceDescriptor::default()
            },
            observed_at_ms: 10,
            kind,
        }
    }

    #[test]
    fn fallback_gate_announces_keyboard_when_enabled() {
        let gate = ServiceKeyboardFallbackGate::new(true);

        let initial = gate.initial_events(20);

        assert_eq!(initial.len(), 1);
        assert!(matches!(initial[0].kind, GilrsInputEventKind::Connected));
        assert_eq!(initial[0].device.device_id, KEYBOARD_FALLBACK_DEVICE_ID);
    }

    #[test]
    fn fallback_gate_suppresses_keyboard_after_first_physical_connect() {
        let mut gate = ServiceKeyboardFallbackGate::new(true);

        let transformed =
            gate.transform_event(physical_event("pad-a", GilrsInputEventKind::Connected), 30);

        assert_eq!(transformed.len(), 2);
        assert!(matches!(
            transformed[0].kind,
            GilrsInputEventKind::Disconnected
        ));
        assert_eq!(transformed[0].device.device_id, KEYBOARD_FALLBACK_DEVICE_ID);
        assert_eq!(transformed[1].device.device_id, "pad-a");
    }

    #[test]
    fn fallback_gate_restores_keyboard_after_last_physical_disconnect() {
        let mut gate = ServiceKeyboardFallbackGate::new(true);
        let _ = gate.transform_event(physical_event("pad-a", GilrsInputEventKind::Connected), 30);

        let transformed = gate.transform_event(
            physical_event("pad-a", GilrsInputEventKind::Disconnected),
            40,
        );

        assert_eq!(transformed.len(), 2);
        assert_eq!(transformed[0].device.device_id, "pad-a");
        assert!(matches!(
            transformed[1].kind,
            GilrsInputEventKind::Connected
        ));
        assert_eq!(transformed[1].device.device_id, KEYBOARD_FALLBACK_DEVICE_ID);
    }

    #[test]
    fn fallback_gate_drops_keyboard_input_while_suppressed() {
        let mut gate = ServiceKeyboardFallbackGate::new(true);
        let _ = gate.transform_event(physical_event("pad-a", GilrsInputEventKind::Connected), 30);

        let transformed = gate.transform_event(
            keyboard_event(GilrsInputEventKind::ButtonChanged {
                index: 0,
                value: 1.0,
            }),
            40,
        );

        assert!(transformed.is_empty());
    }
}
