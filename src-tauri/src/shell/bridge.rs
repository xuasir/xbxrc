use crate::event_bridge;
use crate::mods::xbxengine::events;
use std::sync::{Arc, Mutex as StdMutex};
use tauri::AppHandle;
use xbxengine_protocol::XbxEngineRuntimeEventDto;

#[derive(Clone, Debug, Default)]
pub struct TauriEngineWindowState {
    pub title: String,
    pub surface_id: Option<String>,
    pub video_size: Option<(u32, u32)>,
    pub runtime_phase: Option<xbxengine_protocol::XbxEngineRuntimePhaseDto>,
    pub transport_state: Option<xbxengine_protocol::XbxEngineTransportStateDto>,
    pub last_error: Option<String>,
}

pub trait TauriEngineWindowHost: Send {
    fn open_window(&mut self, title: &str);
    fn apply_event(&mut self, event: &XbxEngineRuntimeEventDto);
    fn snapshot(&self) -> TauriEngineWindowState;
}

#[derive(Default)]
pub struct NoopTauriEngineWindowHost {
    state: TauriEngineWindowState,
}

impl TauriEngineWindowHost for NoopTauriEngineWindowHost {
    fn open_window(&mut self, title: &str) {
        self.state.title = title.to_string();
    }

    fn apply_event(&mut self, _event: &XbxEngineRuntimeEventDto) {}

    fn snapshot(&self) -> TauriEngineWindowState {
        self.state.clone()
    }
}

pub struct TauriEngineEventBridge {
    pub app_handle: AppHandle,
    pub state: TauriEngineWindowState,
    pub last_runtime_event: Arc<StdMutex<Option<serde_json::Value>>>,
}

impl TauriEngineWindowHost for TauriEngineEventBridge {
    fn open_window(&mut self, title: &str) {
        self.state.title = title.to_string();
    }

    fn apply_event(&mut self, event: &XbxEngineRuntimeEventDto) {
        match event {
            XbxEngineRuntimeEventDto::RuntimePhaseChanged { phase } => {
                self.state.runtime_phase = Some(phase.clone());
            }
            XbxEngineRuntimeEventDto::TransportConnectionStateChanged { state } => {
                self.state.transport_state = Some(state.clone());
            }
            XbxEngineRuntimeEventDto::MediaSurfaceReady { surface_id } => {
                self.state.surface_id = Some(surface_id.clone());
            }
            XbxEngineRuntimeEventDto::MediaVideoReady { width, height } => {
                self.state.video_size = Some((*width, *height));
            }
            _ => {}
        }

        if let Some(payload) = events::map_runtime_event(event) {
            if let Ok(mut lock) = self.last_runtime_event.lock() {
                *lock = Some(payload.clone());
            }
            let _ = event_bridge::emit_json(
                &self.app_handle,
                events::STREAMING_XBXENGINE_RUNTIME_EVENT_CHANNEL,
                &payload,
            );
        }
    }

    fn snapshot(&self) -> TauriEngineWindowState {
        self.state.clone()
    }
}
