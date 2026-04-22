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

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OhMyGamepadInputPolicyDto {
    #[default]
    Shared,
    UiOnly,
    StreamOnly,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GamepadSlotBindingDto {
    pub slot: crate::GamepadSlotDto,
    pub mode: OhMyGamepadBindingModeDto,
    pub device_ids: Vec<String>,
}

pub type LogicalPadBindingDto = GamepadSlotBindingDto;
