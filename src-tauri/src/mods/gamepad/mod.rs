pub mod events;
pub mod rpc;
pub mod service;
mod types;

pub use service::GamepadService;

use ohmygamepad_protocol::{
    OhMyGamepadRumbleRequestDto, OhMyGamepadRumbleResultDto, OhMyGamepadRumbleTargetDto,
};
use std::sync::Arc;
pub use types::GamepadRuntimeSnapshotDto;

pub trait GamepadProvider: Send + Sync {
    fn start_backend(&self) -> Result<GamepadRuntimeSnapshotDto, String>;
    fn stop_backend(&self) -> Result<(), String>;
    fn play_rumble(
        &self,
        request: OhMyGamepadRumbleRequestDto,
    ) -> Result<OhMyGamepadRumbleResultDto, String>;
    fn stop_rumble(
        &self,
        target: OhMyGamepadRumbleTargetDto,
    ) -> Result<OhMyGamepadRumbleResultDto, String>;
    fn shutdown(&self) {
        let _ = self.stop_backend();
    }
}

pub type GamepadProviderRef = Arc<dyn GamepadProvider>;
