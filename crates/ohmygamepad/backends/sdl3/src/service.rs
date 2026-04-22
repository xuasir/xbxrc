pub use ohmygamepad_gilrs::{
    MultiControllerSamplingMode, MultiControllerSamplingStrategy,
    OhMyGamepadDesktopKeyboardListenerConfig, OhMyGamepadService as Sdl3Service,
    OhMyGamepadServiceConfig as Sdl3ServiceConfig, OhMyGamepadServiceError as Sdl3ServiceError,
    SimulatedGamepadDescriptor,
};

pub type OhMyGamepadService = Sdl3Service;
pub type OhMyGamepadServiceConfig = Sdl3ServiceConfig;
