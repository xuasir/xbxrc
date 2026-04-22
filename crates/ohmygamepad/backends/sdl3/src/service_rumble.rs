use ohmygamepad_core::{HapticsProvider, HapticsProviderError, InputRuntimeError};
use ohmygamepad_protocol::{
    OhMyGamepadBackendKindDto, OhMyGamepadDeviceDto, OhMyGamepadRumbleEffectDto,
    OhMyGamepadRumbleRejectionReasonDto, OhMyGamepadRumbleResultDto, OhMyGamepadRumbleTargetDto,
    OhMyGamepadRuntimeSnapshotDto,
};

use crate::Sdl3RumbleHandle;

pub(crate) trait ServiceRumbleBackend: Send {
    fn play_rumble(
        &self,
        device_ids: &[String],
        effect: &OhMyGamepadRumbleEffectDto,
    ) -> Result<(), InputRuntimeError>;

    fn stop_rumble(&self, device_ids: &[String]) -> Result<(), InputRuntimeError>;
}

impl ServiceRumbleBackend for Sdl3RumbleHandle {
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
        OhMyGamepadRumbleTargetDto::Slot { slot } => snapshot
            .slots
            .iter()
            .find(|pad| pad.slot == *slot)
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
    if let Some(default_device_id) = snapshot.haptics.default_device_id.as_ref() {
        return vec![default_device_id.clone()];
    }

    if let Some(device_ids) = snapshot.slots.iter().find_map(|pad| {
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
        .find(|device| {
            device.connected
                && (device.sdl3_capabilities.supports_rumble
                    || device.sdl3_capabilities.supports_trigger_rumble)
        })
        .map(|device| vec![device.device_id.clone()])
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
    if device.sdl3_capabilities.supports_rumble || device.sdl3_capabilities.supports_trigger_rumble
    {
        return true;
    }

    has_rumble_backend && device.backend == Some(OhMyGamepadBackendKindDto::Sdl3)
}
