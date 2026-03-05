use ohmygamepad_core::InputRuntimeError;
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

    // `gilrs` 在部分桌面平台上会保守地报告 FF 能力。
    // 只要当前确实有 rumble backend，就允许真实 gilrs 设备继续尝试下发。
    has_rumble_backend && device.backend == Some(OhMyGamepadBackendKindDto::Gilrs)
}

#[cfg(test)]
mod tests {
    use ohmygamepad_protocol::{
        OhMyGamepadBackendKindDto, OhMyGamepadCapabilityFlagsDto,
        OhMyGamepadRumbleRejectionReasonDto,
    };

    use super::{prepare_rumble_dispatch, PreparedRumbleRequest};

    fn device(device_id: &str) -> ohmygamepad_protocol::OhMyGamepadDeviceDto {
        ohmygamepad_protocol::OhMyGamepadDeviceDto {
            device_id: device_id.to_owned(),
            name: device_id.to_owned(),
            backend: Some(OhMyGamepadBackendKindDto::Gilrs),
            connection: None,
            vendor_id: None,
            product_id: None,
            connected: true,
            last_seen_at_ms: 0,
            capabilities: OhMyGamepadCapabilityFlagsDto::default(),
        }
    }

    fn rumble_device(device_id: &str) -> ohmygamepad_protocol::OhMyGamepadDeviceDto {
        let mut device = device(device_id);
        device.capabilities.basic_rumble = true;
        device
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
    fn prepare_rumble_dispatch_allows_gilrs_fallback_with_backend() {
        let prepared = prepare_rumble_dispatch(vec![device("pad-a")], true);

        let PreparedRumbleRequest::Dispatch(dispatch) = prepared else {
            panic!("gilrs device should dispatch when backend exists");
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
