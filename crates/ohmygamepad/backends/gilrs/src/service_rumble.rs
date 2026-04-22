use ohmygamepad_core::{HapticsProvider, HapticsProviderError, InputRuntimeError};
use ohmygamepad_protocol::{
    OhMyGamepadBackendKindDto, OhMyGamepadDeviceDto, OhMyGamepadRumbleEffectDto,
    OhMyGamepadRumbleRejectionReasonDto, OhMyGamepadRumbleResultDto, OhMyGamepadRumbleTargetDto,
    OhMyGamepadRuntimeSnapshotDto,
};

use crate::GilrsRumbleHandle;

pub(crate) trait ServiceRumbleBackend: Send {
    fn play_rumble(
        &self,
        device_ids: &[String],
        effect: &OhMyGamepadRumbleEffectDto,
    ) -> Result<(), InputRuntimeError>;

    fn stop_rumble(&self, device_ids: &[String]) -> Result<(), InputRuntimeError>;
}

impl ServiceRumbleBackend for GilrsRumbleHandle {
    fn play_rumble(
        &self,
        device_ids: &[String],
        effect: &OhMyGamepadRumbleEffectDto,
    ) -> Result<(), InputRuntimeError> {
        self.play_rumble(device_ids.to_vec(), effect.clone())
            .map_err(|_| InputRuntimeError::CommandChannelClosed)
    }

    fn stop_rumble(&self, device_ids: &[String]) -> Result<(), InputRuntimeError> {
        self.stop_rumble(device_ids.to_vec())
            .map_err(|_| InputRuntimeError::CommandChannelClosed)
    }
}

pub(crate) fn rumble_backend_from_haptics_provider(
    provider: Box<dyn HapticsProvider>,
) -> Box<dyn ServiceRumbleBackend> {
    Box::new(HapticsProviderRumbleBackend { provider })
}

struct HapticsProviderRumbleBackend {
    provider: Box<dyn HapticsProvider>,
}

impl ServiceRumbleBackend for HapticsProviderRumbleBackend {
    fn play_rumble(
        &self,
        device_ids: &[String],
        effect: &OhMyGamepadRumbleEffectDto,
    ) -> Result<(), InputRuntimeError> {
        self.provider
            .play_rumble(device_ids, effect)
            .map_err(map_haptics_provider_error)
    }

    fn stop_rumble(&self, device_ids: &[String]) -> Result<(), InputRuntimeError> {
        self.provider
            .stop_rumble(device_ids)
            .map_err(map_haptics_provider_error)
    }
}

fn map_haptics_provider_error(error: HapticsProviderError) -> InputRuntimeError {
    match error {
        HapticsProviderError::Unsupported => InputRuntimeError::HapticsUnavailable,
        HapticsProviderError::TransportClosed => InputRuntimeError::HapticsTransportFailed,
    }
}

pub(crate) struct PreparedRumbleDispatch {
    device_ids: Vec<String>,
}

impl PreparedRumbleDispatch {
    pub(crate) fn device_ids(&self) -> &[String] {
        &self.device_ids
    }

    pub(crate) fn into_result(self) -> OhMyGamepadRumbleResultDto {
        OhMyGamepadRumbleResultDto {
            accepted: true,
            reason: None,
            resolved_device_ids: self.device_ids,
        }
    }
}

pub(crate) enum PreparedRumbleRequest {
    Dispatch(PreparedRumbleDispatch),
    Rejected(OhMyGamepadRumbleResultDto),
}

pub(crate) fn resolve_connected_target_devices(
    snapshot: &OhMyGamepadRuntimeSnapshotDto,
    target: &OhMyGamepadRumbleTargetDto,
    empty_sampling_device_id: &str,
) -> Vec<OhMyGamepadDeviceDto> {
    let resolved_device_ids = match target {
        OhMyGamepadRumbleTargetDto::Auto => {
            resolve_default_target_device_ids(snapshot, empty_sampling_device_id)
        }
        OhMyGamepadRumbleTargetDto::Device { device_id } => snapshot
            .devices
            .iter()
            .find(|device| device.device_id == *device_id && device.connected)
            .map(|device| vec![device.device_id.clone()])
            .unwrap_or_default(),
        OhMyGamepadRumbleTargetDto::LogicalPad { pad_id } => snapshot
            .pads
            .iter()
            .find(|pad| pad.pad_id == *pad_id)
            .map(|pad| {
                pad.device_ids
                    .iter()
                    .filter(|device_id| device_id.as_str() != empty_sampling_device_id)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    };

    snapshot
        .devices
        .iter()
        .filter(|device| {
            device.connected
                && resolved_device_ids
                    .iter()
                    .any(|resolved_id| resolved_id == &device.device_id)
        })
        .cloned()
        .collect()
}

fn resolve_default_target_device_ids(
    snapshot: &OhMyGamepadRuntimeSnapshotDto,
    empty_sampling_device_id: &str,
) -> Vec<String> {
    if let Some(device_ids) = snapshot.pads.iter().find_map(|pad| {
        let resolved = pad
            .device_ids
            .iter()
            .filter(|device_id| device_id.as_str() != empty_sampling_device_id)
            .cloned()
            .collect::<Vec<_>>();
        (!resolved.is_empty()).then_some(resolved)
    }) {
        return device_ids;
    }

    snapshot
        .devices
        .iter()
        .find(|device| device.connected && device.is_default_target)
        .map(|device| vec![device.device_id.clone()])
        .or_else(|| {
            snapshot
                .devices
                .iter()
                .find(|device| {
                    device.connected
                        && (device.effective_capabilities.basic_rumble
                            || device.effective_capabilities.advanced_haptics)
                })
                .map(|device| vec![device.device_id.clone()])
        })
        .unwrap_or_default()
}

pub(crate) fn prepare_rumble_dispatch(
    devices: Vec<OhMyGamepadDeviceDto>,
    has_rumble_backend: bool,
) -> PreparedRumbleRequest {
    if devices.is_empty() {
        return PreparedRumbleRequest::Rejected(OhMyGamepadRumbleResultDto::rejected(
            OhMyGamepadRumbleRejectionReasonDto::TargetNotFound,
            Vec::new(),
        ));
    }

    let resolved_device_ids = devices
        .iter()
        .map(|device| device.device_id.clone())
        .collect::<Vec<_>>();
    let supported_device_ids = devices
        .iter()
        .filter(|device| supports_service_rumble(device, has_rumble_backend))
        .map(|device| device.device_id.clone())
        .collect::<Vec<_>>();

    if supported_device_ids.is_empty() {
        return PreparedRumbleRequest::Rejected(OhMyGamepadRumbleResultDto::rejected(
            OhMyGamepadRumbleRejectionReasonDto::Unsupported,
            resolved_device_ids,
        ));
    }

    if !has_rumble_backend {
        return PreparedRumbleRequest::Rejected(OhMyGamepadRumbleResultDto::rejected(
            OhMyGamepadRumbleRejectionReasonDto::NotImplemented,
            supported_device_ids,
        ));
    }

    PreparedRumbleRequest::Dispatch(PreparedRumbleDispatch {
        device_ids: supported_device_ids,
    })
}

fn supports_service_rumble(device: &OhMyGamepadDeviceDto, has_rumble_backend: bool) -> bool {
    if device.capabilities.basic_rumble || device.capabilities.advanced_haptics {
        return true;
    }

    has_rumble_backend && device.backend == Some(OhMyGamepadBackendKindDto::Sdl3)
}

#[cfg(test)]
mod tests {
    use ohmygamepad_protocol::{
        LogicalPadId, LogicalPadSnapshotDto, OhMyGamepadBackendKindDto,
        OhMyGamepadCapabilityFlagsDto, OhMyGamepadRumbleRejectionReasonDto,
        OhMyGamepadRuntimeHapticsDto,
    };

    use super::{prepare_rumble_dispatch, resolve_connected_target_devices, PreparedRumbleRequest};

    fn device(device_id: &str) -> ohmygamepad_protocol::OhMyGamepadDeviceDto {
        ohmygamepad_protocol::OhMyGamepadDeviceDto {
            device_id: device_id.to_owned(),
            name: device_id.to_owned(),
            backend: Some(OhMyGamepadBackendKindDto::Sdl3),
            connection: None,
            vendor_id: None,
            product_id: None,
            connected: true,
            last_seen_at_ms: 0,
            capabilities: OhMyGamepadCapabilityFlagsDto::default(),
            effective_capabilities: OhMyGamepadCapabilityFlagsDto::default(),
            is_default_target: false,
        }
    }

    fn rumble_device(device_id: &str) -> ohmygamepad_protocol::OhMyGamepadDeviceDto {
        let mut device = device(device_id);
        device.capabilities.basic_rumble = true;
        device.effective_capabilities.basic_rumble = true;
        device
    }

    #[test]
    fn auto_target_prefers_first_pad_with_connected_devices() {
        let devices = vec![rumble_device("pad-a"), rumble_device("pad-b")];
        let snapshot = ohmygamepad_protocol::OhMyGamepadRuntimeSnapshotDto {
            devices,
            pads: vec![LogicalPadSnapshotDto {
                pad_id: LogicalPadId::Pad0,
                device_ids: vec!["pad-b".to_owned()],
                sampled_at_ms: 1,
                sample_seq: 1,
                route_target: ohmygamepad_protocol::OhMyGamepadRouteTargetDto::ShellUi,
                state: Default::default(),
            }],
            haptics: OhMyGamepadRuntimeHapticsDto::default(),
            ..Default::default()
        };

        let resolved = resolve_connected_target_devices(
            &snapshot,
            &ohmygamepad_protocol::OhMyGamepadRumbleTargetDto::Auto,
            "__service:none__",
        );

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].device_id, "pad-b");
    }

    #[test]
    fn prepare_rumble_dispatch_rejects_missing_target() {
        let prepared = prepare_rumble_dispatch(Vec::new(), true);

        let PreparedRumbleRequest::Rejected(result) = prepared else {
            panic!("missing target should be rejected");
        };
        assert_eq!(
            result.reason,
            Some(OhMyGamepadRumbleRejectionReasonDto::TargetNotFound)
        );
    }

    #[test]
    fn prepare_rumble_dispatch_allows_sdl3_fallback_with_backend() {
        let prepared = prepare_rumble_dispatch(vec![device("pad-a")], true);

        let PreparedRumbleRequest::Dispatch(dispatch) = prepared else {
            panic!("sdl3 device should dispatch when backend exists");
        };
        assert_eq!(dispatch.device_ids(), ["pad-a".to_owned()]);
    }

    #[test]
    fn prepare_rumble_dispatch_reports_not_implemented_without_backend() {
        let prepared = prepare_rumble_dispatch(vec![rumble_device("pad-a")], false);

        let PreparedRumbleRequest::Rejected(result) = prepared else {
            panic!("missing backend should be rejected");
        };
        assert_eq!(
            result.reason,
            Some(OhMyGamepadRumbleRejectionReasonDto::NotImplemented)
        );
        assert_eq!(result.resolved_device_ids, vec!["pad-a".to_owned()]);
    }
}
