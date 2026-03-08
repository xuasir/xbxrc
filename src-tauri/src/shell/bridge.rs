use crate::event_bridge;
use crate::mods::xbxengine::events;
use std::sync::{Arc, Mutex as StdMutex};
use tauri::AppHandle;
use xbxengine_app::{XbxEngineWindowHost, XbxEngineWindowState};
use xbxengine_protocol::XbxEngineRuntimeEventDto;

pub struct PrintlnHostBridge {
    pub app_handle: AppHandle,
    pub state: XbxEngineWindowState,
    pub last_runtime_event: Arc<StdMutex<Option<serde_json::Value>>>,
}

impl XbxEngineWindowHost for PrintlnHostBridge {
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

    fn snapshot(&self) -> XbxEngineWindowState {
        self.state.clone()
    }
}
