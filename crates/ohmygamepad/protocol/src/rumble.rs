use crate::LogicalPadId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum OhMyGamepadRumbleTargetDto {
    Auto,
    LogicalPad {
        #[serde(rename = "padId")]
        pad_id: LogicalPadId,
    },
    Device {
        #[serde(rename = "deviceId")]
        device_id: String,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OhMyGamepadRumbleEffectDto {
    pub start_delay_ms: u16,
    pub duration_ms: u16,
    pub strong_magnitude: f32,
    pub weak_magnitude: f32,
    pub left_trigger: f32,
    pub right_trigger: f32,
    pub repeat: u8,
}

impl Default for OhMyGamepadRumbleEffectDto {
    fn default() -> Self {
        Self {
            start_delay_ms: 0,
            duration_ms: 120,
            strong_magnitude: 0.0,
            weak_magnitude: 0.0,
            left_trigger: 0.0,
            right_trigger: 0.0,
            repeat: 0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OhMyGamepadRumbleRequestDto {
    pub target: OhMyGamepadRumbleTargetDto,
    pub effect: OhMyGamepadRumbleEffectDto,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OhMyGamepadRumbleRejectionReasonDto {
    TargetNotFound,
    Unsupported,
    NotImplemented,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OhMyGamepadRumbleResultDto {
    pub accepted: bool,
    pub reason: Option<OhMyGamepadRumbleRejectionReasonDto>,
    pub resolved_device_ids: Vec<String>,
}

impl OhMyGamepadRumbleResultDto {
    pub fn rejected(
        reason: OhMyGamepadRumbleRejectionReasonDto,
        resolved_device_ids: Vec<String>,
    ) -> Self {
        Self {
            accepted: false,
            reason: Some(reason),
            resolved_device_ids,
        }
    }
}
