use std::convert::TryFrom;

use ohmygamepad_protocol::{
    LogicalButtonsStateDto, LogicalPadBindingDto, LogicalPadId, LogicalPadSnapshotDto,
    LogicalPadStateDto, LogicalStickDto, MultiControllerSamplingModeDto,
    MultiControllerSamplingStrategyDto, OhMyGamepadBackendKindDto, OhMyGamepadBindingModeDto,
    OhMyGamepadCapabilityFlagsDto, OhMyGamepadConnectionKindDto, OhMyGamepadDeviceDto,
    OhMyGamepadRouteTargetDto, OhMyGamepadRumbleEffectDto, OhMyGamepadRumbleRejectionReasonDto,
    OhMyGamepadRumbleRequestDto, OhMyGamepadRumbleResultDto, OhMyGamepadRumbleTargetDto,
    OhMyGamepadRuntimeSnapshotDto, OhMyGamepadSamplingConfigDto, OhMyGamepadStreamPushModeDto,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TsGamepadBackendKind {
    Gilrs,
    Mock,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TsGamepadConnectionKind {
    Usb,
    Bluetooth,
    WirelessDongle,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum TsLogicalPadId {
    #[serde(rename = "pad-0")]
    Pad0,
    #[serde(rename = "pad-1")]
    Pad1,
    #[serde(rename = "pad-2")]
    Pad2,
    #[serde(rename = "pad-3")]
    Pad3,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TsGamepadBindingMode {
    SingleActive,
    FixedDevice,
    Merged,
    Split,
    LastActiveFailover,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TsGamepadStreamPushMode {
    OnChange,
    FixedRate,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TsGamepadSamplingMode {
    Merge,
    PrimaryPreferred,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TsGamepadRumbleRejectionReason {
    TargetNotFound,
    Unsupported,
    NotImplemented,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TsGamepadCapabilityFlags {
    pub basic_rumble: bool,
    pub advanced_haptics: bool,
    pub battery: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TsGamepadDevice {
    pub device_id: String,
    pub name: String,
    pub backend: Option<TsGamepadBackendKind>,
    pub connection: Option<TsGamepadConnectionKind>,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub connected: bool,
    pub last_seen_at_ms: u64,
    pub capabilities: TsGamepadCapabilityFlags,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TsLogicalStick {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TsLogicalButtonsState {
    pub south: f32,
    pub east: f32,
    pub west: f32,
    pub north: f32,
    pub l1: f32,
    pub r1: f32,
    pub l2: f32,
    pub r2: f32,
    pub l3: f32,
    pub r3: f32,
    pub view: f32,
    pub menu: f32,
    pub home: f32,
    pub dpad_up: f32,
    pub dpad_down: f32,
    pub dpad_left: f32,
    pub dpad_right: f32,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TsLogicalPadState {
    pub buttons: TsLogicalButtonsState,
    pub left_stick: TsLogicalStick,
    pub right_stick: TsLogicalStick,
    pub left_trigger: f32,
    pub right_trigger: f32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum TsGamepadRouteTarget {
    ShellUi,
    StreamSession {
        #[serde(rename = "sessionId")]
        session_id: String,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TsLogicalPadSnapshot {
    pub pad_id: TsLogicalPadId,
    pub device_ids: Vec<String>,
    pub sampled_at_ms: u64,
    pub sample_seq: u64,
    pub route_target: TsGamepadRouteTarget,
    pub state: TsLogicalPadState,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TsLogicalPadBinding {
    pub pad_id: TsLogicalPadId,
    pub mode: TsGamepadBindingMode,
    pub device_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TsGamepadSamplingConfig {
    pub backend_poll_rate_hz: u16,
    pub logical_pad_sample_rate_hz: u16,
    pub ui_push_rate_hz: u16,
    pub stream_push_mode: TsGamepadStreamPushMode,
    pub stream_push_rate_hz: Option<u16>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TsGamepadSamplingStrategy {
    pub mode: TsGamepadSamplingMode,
    pub primary_device_id: Option<String>,
    pub paused_device_ids: Vec<String>,
    pub enable_keyboard_fallback: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum TsGamepadRumbleTarget {
    LogicalPad {
        #[serde(rename = "padId")]
        pad_id: TsLogicalPadId,
    },
    Device {
        #[serde(rename = "deviceId")]
        device_id: String,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TsGamepadRumbleEffect {
    pub start_delay_ms: u16,
    pub duration_ms: u16,
    pub strong_magnitude: f32,
    pub weak_magnitude: f32,
    pub left_trigger: f32,
    pub right_trigger: f32,
    pub repeat: u8,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TsGamepadRumbleRequest {
    pub target: TsGamepadRumbleTarget,
    pub effect: TsGamepadRumbleEffect,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TsGamepadRumbleResult {
    pub accepted: bool,
    pub reason: Option<TsGamepadRumbleRejectionReason>,
    pub resolved_device_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TsGamepadRuntimeSnapshot {
    pub devices: Vec<TsGamepadDevice>,
    pub bindings: Vec<TsLogicalPadBinding>,
    pub route_target: TsGamepadRouteTarget,
    pub sampling: TsGamepadSamplingConfig,
    pub pads: Vec<TsLogicalPadSnapshot>,
}

impl From<OhMyGamepadRuntimeSnapshotDto> for TsGamepadRuntimeSnapshot {
    fn from(value: OhMyGamepadRuntimeSnapshotDto) -> Self {
        Self {
            devices: value.devices.into_iter().map(Into::into).collect(),
            bindings: value.bindings.into_iter().map(Into::into).collect(),
            route_target: value.route_target.into(),
            sampling: value.sampling.into(),
            pads: value.pads.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<OhMyGamepadDeviceDto> for TsGamepadDevice {
    fn from(value: OhMyGamepadDeviceDto) -> Self {
        Self {
            device_id: value.device_id,
            name: value.name,
            backend: value.backend.map(Into::into),
            connection: value.connection.map(Into::into),
            vendor_id: value.vendor_id,
            product_id: value.product_id,
            connected: value.connected,
            last_seen_at_ms: value.last_seen_at_ms,
            capabilities: value.capabilities.into(),
        }
    }
}

impl From<OhMyGamepadCapabilityFlagsDto> for TsGamepadCapabilityFlags {
    fn from(value: OhMyGamepadCapabilityFlagsDto) -> Self {
        Self {
            basic_rumble: value.basic_rumble,
            advanced_haptics: value.advanced_haptics,
            battery: value.battery,
        }
    }
}

impl From<OhMyGamepadBackendKindDto> for TsGamepadBackendKind {
    fn from(value: OhMyGamepadBackendKindDto) -> Self {
        match value {
            OhMyGamepadBackendKindDto::Gilrs => Self::Gilrs,
            OhMyGamepadBackendKindDto::Mock => Self::Mock,
        }
    }
}

impl From<OhMyGamepadConnectionKindDto> for TsGamepadConnectionKind {
    fn from(value: OhMyGamepadConnectionKindDto) -> Self {
        match value {
            OhMyGamepadConnectionKindDto::Usb => Self::Usb,
            OhMyGamepadConnectionKindDto::Bluetooth => Self::Bluetooth,
            OhMyGamepadConnectionKindDto::WirelessDongle => Self::WirelessDongle,
            OhMyGamepadConnectionKindDto::Unknown => Self::Unknown,
        }
    }
}

impl From<LogicalPadSnapshotDto> for TsLogicalPadSnapshot {
    fn from(value: LogicalPadSnapshotDto) -> Self {
        Self {
            pad_id: value.pad_id.into(),
            device_ids: value.device_ids,
            sampled_at_ms: value.sampled_at_ms,
            sample_seq: value.sample_seq,
            route_target: value.route_target.into(),
            state: value.state.into(),
        }
    }
}

impl From<LogicalPadBindingDto> for TsLogicalPadBinding {
    fn from(value: LogicalPadBindingDto) -> Self {
        Self {
            pad_id: value.pad_id.into(),
            mode: value.mode.into(),
            device_ids: value.device_ids,
        }
    }
}

impl From<OhMyGamepadSamplingConfigDto> for TsGamepadSamplingConfig {
    fn from(value: OhMyGamepadSamplingConfigDto) -> Self {
        Self {
            backend_poll_rate_hz: value.backend_poll_rate_hz,
            logical_pad_sample_rate_hz: value.logical_pad_sample_rate_hz,
            ui_push_rate_hz: value.ui_push_rate_hz,
            stream_push_mode: value.stream_push_mode.into(),
            stream_push_rate_hz: value.stream_push_rate_hz,
        }
    }
}

impl From<LogicalPadStateDto> for TsLogicalPadState {
    fn from(value: LogicalPadStateDto) -> Self {
        Self {
            buttons: value.buttons.into(),
            left_stick: value.left_stick.into(),
            right_stick: value.right_stick.into(),
            left_trigger: value.left_trigger,
            right_trigger: value.right_trigger,
        }
    }
}

impl From<LogicalButtonsStateDto> for TsLogicalButtonsState {
    fn from(value: LogicalButtonsStateDto) -> Self {
        Self {
            south: value.south,
            east: value.east,
            west: value.west,
            north: value.north,
            l1: value.l1,
            r1: value.r1,
            l2: value.l2,
            r2: value.r2,
            l3: value.l3,
            r3: value.r3,
            view: value.view,
            menu: value.menu,
            home: value.home,
            dpad_up: value.dpad_up,
            dpad_down: value.dpad_down,
            dpad_left: value.dpad_left,
            dpad_right: value.dpad_right,
        }
    }
}

impl From<LogicalStickDto> for TsLogicalStick {
    fn from(value: LogicalStickDto) -> Self {
        Self {
            x: value.x,
            y: value.y,
        }
    }
}

impl From<OhMyGamepadRouteTargetDto> for TsGamepadRouteTarget {
    fn from(value: OhMyGamepadRouteTargetDto) -> Self {
        match value {
            OhMyGamepadRouteTargetDto::ShellUi => Self::ShellUi,
            OhMyGamepadRouteTargetDto::StreamSession { session_id } => {
                Self::StreamSession { session_id }
            }
        }
    }
}

impl From<LogicalPadId> for TsLogicalPadId {
    fn from(value: LogicalPadId) -> Self {
        match value {
            LogicalPadId::Pad0 => Self::Pad0,
            LogicalPadId::Pad1 => Self::Pad1,
            LogicalPadId::Pad2 => Self::Pad2,
            LogicalPadId::Pad3 => Self::Pad3,
        }
    }
}

impl From<OhMyGamepadBindingModeDto> for TsGamepadBindingMode {
    fn from(value: OhMyGamepadBindingModeDto) -> Self {
        match value {
            OhMyGamepadBindingModeDto::SingleActive => Self::SingleActive,
            OhMyGamepadBindingModeDto::FixedDevice => Self::FixedDevice,
            OhMyGamepadBindingModeDto::Merged => Self::Merged,
            OhMyGamepadBindingModeDto::Split => Self::Split,
            OhMyGamepadBindingModeDto::LastActiveFailover => Self::LastActiveFailover,
        }
    }
}

impl From<OhMyGamepadStreamPushModeDto> for TsGamepadStreamPushMode {
    fn from(value: OhMyGamepadStreamPushModeDto) -> Self {
        match value {
            OhMyGamepadStreamPushModeDto::OnChange => Self::OnChange,
            OhMyGamepadStreamPushModeDto::FixedRate => Self::FixedRate,
        }
    }
}

impl TryFrom<TsGamepadRouteTarget> for OhMyGamepadRouteTargetDto {
    type Error = String;

    fn try_from(value: TsGamepadRouteTarget) -> Result<Self, Self::Error> {
        Ok(match value {
            TsGamepadRouteTarget::ShellUi => Self::ShellUi,
            TsGamepadRouteTarget::StreamSession { session_id } => {
                Self::StreamSession { session_id }
            }
        })
    }
}

impl TryFrom<TsLogicalPadBinding> for LogicalPadBindingDto {
    type Error = String;

    fn try_from(value: TsLogicalPadBinding) -> Result<Self, Self::Error> {
        Ok(Self {
            pad_id: value.pad_id.into(),
            mode: value.mode.into(),
            device_ids: value.device_ids,
        })
    }
}

impl TryFrom<TsGamepadSamplingConfig> for OhMyGamepadSamplingConfigDto {
    type Error = String;

    fn try_from(value: TsGamepadSamplingConfig) -> Result<Self, Self::Error> {
        Ok(Self {
            backend_poll_rate_hz: value.backend_poll_rate_hz,
            logical_pad_sample_rate_hz: value.logical_pad_sample_rate_hz,
            ui_push_rate_hz: value.ui_push_rate_hz,
            stream_push_mode: value.stream_push_mode.into(),
            stream_push_rate_hz: value.stream_push_rate_hz,
        })
    }
}

impl TryFrom<TsGamepadSamplingStrategy> for MultiControllerSamplingStrategyDto {
    type Error = String;

    fn try_from(value: TsGamepadSamplingStrategy) -> Result<Self, Self::Error> {
        Ok(Self {
            mode: value.mode.into(),
            primary_device_id: value.primary_device_id,
            paused_device_ids: value.paused_device_ids,
            enable_keyboard_fallback: value.enable_keyboard_fallback,
        })
    }
}

impl TryFrom<TsGamepadRumbleTarget> for OhMyGamepadRumbleTargetDto {
    type Error = String;

    fn try_from(value: TsGamepadRumbleTarget) -> Result<Self, Self::Error> {
        Ok(match value {
            TsGamepadRumbleTarget::LogicalPad { pad_id } => Self::LogicalPad {
                pad_id: pad_id.into(),
            },
            TsGamepadRumbleTarget::Device { device_id } => Self::Device { device_id },
        })
    }
}

impl TryFrom<TsGamepadRumbleRequest> for OhMyGamepadRumbleRequestDto {
    type Error = String;

    fn try_from(value: TsGamepadRumbleRequest) -> Result<Self, Self::Error> {
        Ok(Self {
            target: value.target.try_into()?,
            effect: value.effect.into(),
        })
    }
}

impl From<OhMyGamepadRumbleResultDto> for TsGamepadRumbleResult {
    fn from(value: OhMyGamepadRumbleResultDto) -> Self {
        Self {
            accepted: value.accepted,
            reason: value.reason.map(Into::into),
            resolved_device_ids: value.resolved_device_ids,
        }
    }
}

impl From<OhMyGamepadRumbleEffectDto> for TsGamepadRumbleEffect {
    fn from(value: OhMyGamepadRumbleEffectDto) -> Self {
        Self {
            start_delay_ms: value.start_delay_ms,
            duration_ms: value.duration_ms,
            strong_magnitude: value.strong_magnitude,
            weak_magnitude: value.weak_magnitude,
            left_trigger: value.left_trigger,
            right_trigger: value.right_trigger,
            repeat: value.repeat,
        }
    }
}

impl From<TsGamepadRumbleEffect> for OhMyGamepadRumbleEffectDto {
    fn from(value: TsGamepadRumbleEffect) -> Self {
        Self {
            start_delay_ms: value.start_delay_ms,
            duration_ms: value.duration_ms,
            strong_magnitude: value.strong_magnitude,
            weak_magnitude: value.weak_magnitude,
            left_trigger: value.left_trigger,
            right_trigger: value.right_trigger,
            repeat: value.repeat,
        }
    }
}

impl From<TsLogicalPadId> for LogicalPadId {
    fn from(value: TsLogicalPadId) -> Self {
        match value {
            TsLogicalPadId::Pad0 => Self::Pad0,
            TsLogicalPadId::Pad1 => Self::Pad1,
            TsLogicalPadId::Pad2 => Self::Pad2,
            TsLogicalPadId::Pad3 => Self::Pad3,
        }
    }
}

impl From<TsGamepadBindingMode> for OhMyGamepadBindingModeDto {
    fn from(value: TsGamepadBindingMode) -> Self {
        match value {
            TsGamepadBindingMode::SingleActive => Self::SingleActive,
            TsGamepadBindingMode::FixedDevice => Self::FixedDevice,
            TsGamepadBindingMode::Merged => Self::Merged,
            TsGamepadBindingMode::Split => Self::Split,
            TsGamepadBindingMode::LastActiveFailover => Self::LastActiveFailover,
        }
    }
}

impl From<TsGamepadStreamPushMode> for OhMyGamepadStreamPushModeDto {
    fn from(value: TsGamepadStreamPushMode) -> Self {
        match value {
            TsGamepadStreamPushMode::OnChange => Self::OnChange,
            TsGamepadStreamPushMode::FixedRate => Self::FixedRate,
        }
    }
}

impl From<TsGamepadSamplingMode> for MultiControllerSamplingModeDto {
    fn from(value: TsGamepadSamplingMode) -> Self {
        match value {
            TsGamepadSamplingMode::Merge => Self::Merge,
            TsGamepadSamplingMode::PrimaryPreferred => Self::PrimaryPreferred,
        }
    }
}

impl From<OhMyGamepadRumbleRejectionReasonDto> for TsGamepadRumbleRejectionReason {
    fn from(value: OhMyGamepadRumbleRejectionReasonDto) -> Self {
        match value {
            OhMyGamepadRumbleRejectionReasonDto::TargetNotFound => Self::TargetNotFound,
            OhMyGamepadRumbleRejectionReasonDto::Unsupported => Self::Unsupported,
            OhMyGamepadRumbleRejectionReasonDto::NotImplemented => Self::NotImplemented,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        TsGamepadRouteTarget, TsGamepadRumbleRequest, TsGamepadRumbleResult,
        TsGamepadRuntimeSnapshot, TsGamepadSamplingConfig, TsGamepadSamplingStrategy,
        TsLogicalPadBinding,
    };
    use ohmygamepad_protocol::{
        LogicalPadBindingDto, LogicalPadId, LogicalPadSnapshotDto, LogicalPadStateDto,
        MultiControllerSamplingModeDto, MultiControllerSamplingStrategyDto,
        OhMyGamepadBindingModeDto, OhMyGamepadRouteTargetDto, OhMyGamepadRumbleRejectionReasonDto,
        OhMyGamepadRumbleRequestDto, OhMyGamepadRumbleResultDto, OhMyGamepadRumbleTargetDto,
        OhMyGamepadRuntimeSnapshotDto, OhMyGamepadSamplingConfigDto, OhMyGamepadStreamPushModeDto,
    };

    #[test]
    fn snapshot_serializes_to_typescript_contract_shape() {
        let snapshot = TsGamepadRuntimeSnapshot::from(OhMyGamepadRuntimeSnapshotDto {
            devices: vec![],
            bindings: vec![LogicalPadBindingDto {
                pad_id: LogicalPadId::Pad0,
                mode: OhMyGamepadBindingModeDto::Merged,
                device_ids: vec!["pad-a".to_owned()],
            }],
            route_target: OhMyGamepadRouteTargetDto::StreamSession {
                session_id: "stream-1".to_owned(),
            },
            sampling: OhMyGamepadSamplingConfigDto {
                backend_poll_rate_hz: 250,
                logical_pad_sample_rate_hz: 120,
                ui_push_rate_hz: 60,
                stream_push_mode: OhMyGamepadStreamPushModeDto::OnChange,
                stream_push_rate_hz: None,
            },
            pads: vec![LogicalPadSnapshotDto {
                pad_id: LogicalPadId::Pad0,
                device_ids: vec!["pad-a".to_owned()],
                sampled_at_ms: 42,
                sample_seq: 2,
                route_target: OhMyGamepadRouteTargetDto::ShellUi,
                state: LogicalPadStateDto::default(),
            }],
        });

        let json = serde_json::to_value(snapshot).expect("snapshot should serialize");
        assert_eq!(json["bindings"][0]["padId"], "pad-0");
        assert_eq!(json["routeTarget"]["kind"], "stream-session");
        assert_eq!(json["routeTarget"]["sessionId"], "stream-1");
        assert_eq!(json["sampling"]["backendPollRateHz"], 250);
        assert_eq!(json["pads"][0]["sampleSeq"], 2);
    }

    #[test]
    fn route_target_deserializes_from_typescript_contract_shape() {
        let parsed: TsGamepadRouteTarget = serde_json::from_value(serde_json::json!({
            "kind": "stream-session",
            "sessionId": "stream-2"
        }))
        .expect("route target should parse");

        let target =
            OhMyGamepadRouteTargetDto::try_from(parsed).expect("route target should convert");
        assert_eq!(
            target,
            OhMyGamepadRouteTargetDto::StreamSession {
                session_id: "stream-2".to_owned(),
            }
        );
    }

    #[test]
    fn logical_binding_deserializes_from_typescript_contract_shape() {
        let parsed: TsLogicalPadBinding = serde_json::from_value(serde_json::json!({
            "padId": "pad-1",
            "mode": "fixed-device",
            "deviceIds": ["device-a"]
        }))
        .expect("binding should parse");

        let binding = LogicalPadBindingDto::try_from(parsed).expect("binding should convert");
        assert_eq!(binding.pad_id, LogicalPadId::Pad1);
        assert_eq!(binding.mode, OhMyGamepadBindingModeDto::FixedDevice);
        assert_eq!(binding.device_ids, vec!["device-a".to_owned()]);
    }

    #[test]
    fn sampling_deserializes_from_typescript_contract_shape() {
        let parsed: TsGamepadSamplingConfig = serde_json::from_value(serde_json::json!({
            "backendPollRateHz": 500,
            "logicalPadSampleRateHz": 250,
            "uiPushRateHz": 60,
            "streamPushMode": "fixed-rate",
            "streamPushRateHz": 120
        }))
        .expect("sampling config should parse");

        let sampling =
            OhMyGamepadSamplingConfigDto::try_from(parsed).expect("sampling should convert");
        assert_eq!(sampling.backend_poll_rate_hz, 500);
        assert_eq!(sampling.logical_pad_sample_rate_hz, 250);
        assert_eq!(
            sampling.stream_push_mode,
            OhMyGamepadStreamPushModeDto::FixedRate
        );
        assert_eq!(sampling.stream_push_rate_hz, Some(120));
    }

    #[test]
    fn sampling_strategy_round_trips_typescript_contract_shape() {
        let parsed: TsGamepadSamplingStrategy = serde_json::from_value(serde_json::json!({
            "mode": "primary-preferred",
            "primaryDeviceId": "pad-a",
            "pausedDeviceIds": ["pad-b"],
            "enableKeyboardFallback": false
        }))
        .expect("sampling strategy should parse");

        let strategy =
            MultiControllerSamplingStrategyDto::try_from(parsed).expect("strategy should convert");
        assert_eq!(
            strategy.mode,
            MultiControllerSamplingModeDto::PrimaryPreferred
        );
        assert_eq!(strategy.primary_device_id, Some("pad-a".to_owned()));
        assert_eq!(strategy.paused_device_ids, vec!["pad-b".to_owned()]);
        assert!(!strategy.enable_keyboard_fallback);
    }

    #[test]
    fn rumble_request_and_result_follow_typescript_contract_shape() {
        let parsed: TsGamepadRumbleRequest = serde_json::from_value(serde_json::json!({
            "target": {
                "kind": "logical-pad",
                "padId": "pad-2"
            },
            "effect": {
                "startDelayMs": 10,
                "durationMs": 120,
                "strongMagnitude": 0.7,
                "weakMagnitude": 0.3,
                "leftTrigger": 0.1,
                "rightTrigger": 0.2,
                "repeat": 1
            }
        }))
        .expect("rumble request should parse");

        let request =
            OhMyGamepadRumbleRequestDto::try_from(parsed).expect("request should convert");
        assert_eq!(
            request.target,
            OhMyGamepadRumbleTargetDto::LogicalPad {
                pad_id: LogicalPadId::Pad2,
            }
        );
        assert!((request.effect.strong_magnitude - 0.7).abs() < 0.0001);

        let result = TsGamepadRumbleResult::from(OhMyGamepadRumbleResultDto::rejected(
            OhMyGamepadRumbleRejectionReasonDto::Unsupported,
            vec!["device-a".to_owned()],
        ));
        let json = serde_json::to_value(result).expect("result should serialize");
        assert_eq!(json["accepted"], false);
        assert_eq!(json["reason"], "unsupported");
        assert_eq!(json["resolvedDeviceIds"][0], "device-a");
    }
}
