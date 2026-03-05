use ohmygamepad_core::{HapticsProvider, HapticsProviderError};
use ohmygamepad_protocol::OhMyGamepadRumbleEffectDto;

/**
 * DualSense 高级触觉后端先保留占位。
 * 后续再把 hidapi、USB/BT report 和 adaptive trigger 细节接进来。
 */
#[derive(Default)]
pub struct DualSenseHapticsProviderPlaceholder;

impl HapticsProvider for DualSenseHapticsProviderPlaceholder {
    fn play_rumble(
        &self,
        _device_ids: &[String],
        _effect: &OhMyGamepadRumbleEffectDto,
    ) -> Result<(), HapticsProviderError> {
        Err(HapticsProviderError::Unsupported)
    }

    fn stop_rumble(&self, _device_ids: &[String]) -> Result<(), HapticsProviderError> {
        Err(HapticsProviderError::Unsupported)
    }
}
