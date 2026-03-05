use crate::LogicalPadId;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OhMyGamepadBindingModeDto {
    #[default]
    SingleActive,
    FixedDevice,
    Merged,
    Split,
    LastActiveFailover,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LogicalPadBindingDto {
    pub pad_id: LogicalPadId,
    pub mode: OhMyGamepadBindingModeDto,
    pub device_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum OhMyGamepadRouteTargetDto {
    #[default]
    ShellUi,
    StreamSession {
        session_id: String,
    },
}
