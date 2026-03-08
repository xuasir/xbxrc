use crate::{
    LogicalPadStateDto, OhMyGamepadKeyboardMappingDto, OhMyGamepadRouteTargetDto,
    OhMyGamepadSamplingConfigDto, OhMyGamepadSamplingPresetDto,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MultiControllerSamplingModeDto {
    #[default]
    Merge,
    PrimaryPreferred,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiControllerSamplingStrategyDto {
    pub mode: MultiControllerSamplingModeDto,
    pub primary_device_id: Option<String>,
    pub paused_device_ids: Vec<String>,
    pub enable_keyboard_fallback: bool,
}

impl Default for MultiControllerSamplingStrategyDto {
    fn default() -> Self {
        Self {
            mode: MultiControllerSamplingModeDto::Merge,
            primary_device_id: None,
            paused_device_ids: Vec::new(),
            enable_keyboard_fallback: true,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulatedGamepadDescriptorDto {
    pub device_id: String,
    pub name: String,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum OhMyGamepadServiceCommandDto {
    RefreshSnapshot,
    SetRouteTarget {
        target: OhMyGamepadRouteTargetDto,
    },
    UpdateSampling {
        sampling: OhMyGamepadSamplingConfigDto,
    },
    SetSamplingPreset {
        preset: OhMyGamepadSamplingPresetDto,
    },
    SetSamplingStrategy {
        strategy: MultiControllerSamplingStrategyDto,
    },
    SetPrimarySamplingDevice {
        device_id: Option<String>,
    },
    PauseSamplingDevice {
        device_id: String,
    },
    ResumeSamplingDevice {
        device_id: String,
    },
    ConnectSimulatedGamepad {
        descriptor: SimulatedGamepadDescriptorDto,
    },
    DisconnectSimulatedGamepad {
        device_id: String,
    },
    SubmitSimulatedState {
        device_id: String,
        state: LogicalPadStateDto,
    },
    SubmitKeyboardState {
        state: LogicalPadStateDto,
    },
    ReplaceKeyboardMapping {
        mapping: OhMyGamepadKeyboardMappingDto,
    },
}
