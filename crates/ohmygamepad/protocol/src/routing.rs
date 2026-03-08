use crate::LogicalPadId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OhMyGamepadBindingModeDto {
    #[default]
    SingleActive,
    FixedDevice,
    Merged,
    Split,
    LastActiveFailover,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogicalPadBindingDto {
    pub pad_id: LogicalPadId,
    pub mode: OhMyGamepadBindingModeDto,
    pub device_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum OhMyGamepadRouteTargetDto {
    #[default]
    ShellUi,
    StreamSession {
        #[serde(rename = "sessionId")]
        session_id: String,
    },
}
