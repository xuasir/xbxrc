use input_dto::{GamepadRouteTargetDto, GamepadSamplingConfigDto, LogicalPadBindingDto};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DesktopBackendPreference {
    #[default]
    GilrsPreferred,
    Mock,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct InputCoreConfig {
    pub backend_preference: DesktopBackendPreference,
    pub sampling: GamepadSamplingConfigDto,
    pub bindings: Vec<LogicalPadBindingDto>,
    pub route_target: GamepadRouteTargetDto,
}
