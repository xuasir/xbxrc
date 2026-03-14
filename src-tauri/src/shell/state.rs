use crate::mods;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone, Debug)]
pub struct StartupFlagsState {
    pub fullscreen: bool,
    pub auto_connect: String,
}

#[derive(Clone)]
pub struct AppState {
    pub app_state: mods::app_state::AppStateProviderRef,
    pub auth: mods::auth::AuthProviderRef,
    pub config: mods::config::ConfigProviderRef,
    pub data: mods::data::DataProviderRef,
    pub streaming: mods::streaming::StreamingServiceRef,
    pub runtime_trace: mods::runtime_trace::RuntimeTraceRecorderRef,
    pub xbxengine: mods::xbxengine::XbxEngineProviderRef,
    pub gamepad: mods::gamepad::GamepadProviderRef,
    pub startup_flags: Arc<RwLock<StartupFlagsState>>,
    pub is_quitting: Arc<AtomicBool>,
}
