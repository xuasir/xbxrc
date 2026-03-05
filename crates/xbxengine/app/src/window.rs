use std::sync::{Arc, Mutex};

use xbxengine_protocol::{
    XbxEngineRuntimeEventDto, XbxEngineRuntimePhaseDto, XbxEngineTransportStateDto,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct XbxEngineWindowState {
    pub title: String,
    pub surface_id: Option<String>,
    pub video_size: Option<(u32, u32)>,
    pub runtime_phase: Option<XbxEngineRuntimePhaseDto>,
    pub transport_state: Option<XbxEngineTransportStateDto>,
    pub last_error: Option<String>,
}

pub trait XbxEngineWindowHost: Send {
    fn open_window(&mut self, title: &str);
    fn apply_event(&mut self, event: &XbxEngineRuntimeEventDto);
    fn snapshot(&self) -> XbxEngineWindowState;
}

#[derive(Default)]
pub struct NoopXbxEngineWindowHost {
    state: XbxEngineWindowState,
}

impl XbxEngineWindowHost for NoopXbxEngineWindowHost {
    fn open_window(&mut self, title: &str) {
        self.state.title = title.to_string();
    }

    fn apply_event(&mut self, event: &XbxEngineRuntimeEventDto) {
        apply_runtime_event(&mut self.state, event);
    }

    fn snapshot(&self) -> XbxEngineWindowState {
        self.state.clone()
    }
}

#[derive(Clone, Default)]
pub struct SharedXbxEngineWindowState {
    inner: Arc<Mutex<XbxEngineWindowState>>,
}

impl SharedXbxEngineWindowState {
    pub fn snapshot(&self) -> XbxEngineWindowState {
        self.inner
            .lock()
            .map(|state| state.clone())
            .unwrap_or_else(|_| XbxEngineWindowState::default())
    }

    pub fn apply_event(&self, event: &XbxEngineRuntimeEventDto) {
        if let Ok(mut state) = self.inner.lock() {
            apply_runtime_event(&mut state, event);
        }
    }

    pub fn open_window(&self, title: &str) {
        if let Ok(mut state) = self.inner.lock() {
            state.title = title.to_string();
        }
    }
}

#[derive(Clone, Default)]
pub struct SharedStateXbxEngineWindowHost {
    state: SharedXbxEngineWindowState,
}

impl SharedStateXbxEngineWindowHost {
    pub fn new(state: SharedXbxEngineWindowState) -> Self {
        Self { state }
    }
}

impl XbxEngineWindowHost for SharedStateXbxEngineWindowHost {
    fn open_window(&mut self, title: &str) {
        self.state.open_window(title);
    }

    fn apply_event(&mut self, event: &XbxEngineRuntimeEventDto) {
        self.state.apply_event(event);
    }

    fn snapshot(&self) -> XbxEngineWindowState {
        self.state.snapshot()
    }
}

pub fn build_window_title(state: &XbxEngineWindowState) -> String {
    let mut title = if state.title.is_empty() {
        "Rust Stream Window".to_string()
    } else {
        state.title.clone()
    };

    if let Some((width, height)) = state.video_size {
        title.push_str(&format!(" | {width}x{height}"));
    }

    if let Some(phase) = &state.runtime_phase {
        title.push_str(&format!(" | {phase:?}"));
    }

    if let Some(transport_state) = &state.transport_state {
        title.push_str(&format!(" | {transport_state:?}"));
    }

    if let Some(error) = &state.last_error {
        title.push_str(&format!(" | error:{error}"));
    }

    title
}

fn apply_runtime_event(state: &mut XbxEngineWindowState, event: &XbxEngineRuntimeEventDto) {
    match event {
        XbxEngineRuntimeEventDto::RuntimePhaseChanged { phase } => {
            state.runtime_phase = Some(phase.clone());
        }
        XbxEngineRuntimeEventDto::TransportConnectionStateChanged {
            state: transport_state,
        } => {
            state.transport_state = Some(transport_state.clone());
        }
        XbxEngineRuntimeEventDto::MediaSurfaceReady { surface_id } => {
            state.surface_id = Some(surface_id.clone());
        }
        XbxEngineRuntimeEventDto::MediaVideoReady { width, height } => {
            state.video_size = Some((*width, *height));
        }
        XbxEngineRuntimeEventDto::ErrorReported { code, message } => {
            state.last_error = Some(format!("{code}:{message}"));
        }
        _ => {}
    }
}
