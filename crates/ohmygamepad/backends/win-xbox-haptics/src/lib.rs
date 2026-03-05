use ohmygamepad_core::{HapticsProvider, HapticsProviderError};
use ohmygamepad_protocol::OhMyGamepadRumbleEffectDto;

/**
 * Windows Xbox 高级震动后端先占位。
 * 这里先不引入 WinRT/GameInput 依赖，避免在非 Windows 平台污染 workspace。
 */
#[derive(Default)]
pub struct WindowsXboxHapticsProviderPlaceholder;

impl HapticsProvider for WindowsXboxHapticsProviderPlaceholder {
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
