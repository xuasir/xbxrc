use std::{
    collections::VecDeque,
    sync::mpsc::{Receiver, TryRecvError},
};

use crate::{service_keyboard::ServiceKeyboardFallbackGate, Sdl3InputEvent, Sdl3Source};

#[derive(Debug)]
pub(crate) struct OhMyGamepadServiceSource<TPhysical> {
    physical_source: TPhysical,
    command_rx: Receiver<Vec<Sdl3InputEvent>>,
    pending_events: VecDeque<Sdl3InputEvent>,
    keyboard_fallback_gate: ServiceKeyboardFallbackGate,
    now_ms: fn() -> u64,
}

impl<TPhysical> OhMyGamepadServiceSource<TPhysical> {
    pub(crate) fn new(
        physical_source: TPhysical,
        command_rx: Receiver<Vec<Sdl3InputEvent>>,
        keyboard_fallback_enabled: bool,
        now_ms: fn() -> u64,
    ) -> Self {
        let keyboard_fallback_gate = ServiceKeyboardFallbackGate::new(keyboard_fallback_enabled);
        let pending_events = keyboard_fallback_gate
            .initial_events(now_ms())
            .into_iter()
            .collect::<VecDeque<_>>();

        Self {
            physical_source,
            command_rx,
            pending_events,
            keyboard_fallback_gate,
            now_ms,
        }
    }

    fn enqueue_command_events(&mut self) {
        loop {
            match self.command_rx.try_recv() {
                Ok(events) => {
                    for event in events {
                        let transformed = self
                            .keyboard_fallback_gate
                            .transform_event(event, (self.now_ms)());
                        self.pending_events.extend(transformed);
                    }
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
    }
}

impl<TPhysical> Sdl3Source for OhMyGamepadServiceSource<TPhysical>
where
    TPhysical: Sdl3Source,
{
    fn next_event(&mut self) -> Option<Sdl3InputEvent> {
        self.enqueue_command_events();
        if let Some(event) = self.pending_events.pop_front() {
            return Some(event);
        }

        let event = self.physical_source.next_event()?;
        let mut transformed = self
            .keyboard_fallback_gate
            .transform_event(event, (self.now_ms)())
            .into_iter();
        let first = transformed.next();
        self.pending_events.extend(transformed);
        first
    }
}
