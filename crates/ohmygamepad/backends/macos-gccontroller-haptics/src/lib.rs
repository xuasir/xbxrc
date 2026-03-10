use ohmygamepad_core::{HapticsProvider, HapticsProviderError};
use ohmygamepad_protocol::OhMyGamepadRumbleEffectDto;

/**
 * macOS 的 Xbox 蓝牙震动主线明确走 GameController + Core Haptics。
 * 这里先把 crate/类型边界固定下来，后续再接 objc2 GameController/CoreHaptics。
 */
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MacosGcControllerHapticsConfig {
    pub bluetooth_only: bool,
    pub prefer_primary_controller: bool,
}

impl Default for MacosGcControllerHapticsConfig {
    fn default() -> Self {
        Self {
            bluetooth_only: true,
            prefer_primary_controller: true,
        }
    }
}

pub struct MacosGcControllerHapticsProvider {
    config: MacosGcControllerHapticsConfig,
}

impl MacosGcControllerHapticsProvider {
    pub fn new(config: MacosGcControllerHapticsConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &MacosGcControllerHapticsConfig {
        &self.config
    }
}

impl Default for MacosGcControllerHapticsProvider {
    fn default() -> Self {
        Self::new(MacosGcControllerHapticsConfig::default())
    }
}

impl HapticsProvider for MacosGcControllerHapticsProvider {
    fn play_rumble(
        &self,
        device_ids: &[String],
        effect: &OhMyGamepadRumbleEffectDto,
    ) -> Result<(), HapticsProviderError> {
        platform::play_rumble(&self.config, device_ids, effect)
    }

    fn stop_rumble(&self, device_ids: &[String]) -> Result<(), HapticsProviderError> {
        platform::stop_rumble(&self.config, device_ids)
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use objc2::{msg_send, rc::Retained, sel, AnyThread};
    use objc2_core_haptics::{
        CHHapticEngine, CHHapticEvent, CHHapticEventParameter,
        CHHapticEventParameterIDHapticIntensity, CHHapticEventParameterIDHapticSharpness,
        CHHapticEventTypeHapticContinuous, CHHapticPattern, CHHapticPatternPlayer,
        CHHapticTimeImmediate,
    };
    use objc2_foundation::NSArray;
    use objc2_game_controller::{GCController, GCDeviceHaptics, GCHapticsLocalityDefault};
    use ohmygamepad_core::HapticsProviderError;
    use ohmygamepad_protocol::OhMyGamepadRumbleEffectDto;

    use super::MacosGcControllerHapticsConfig;

    pub(super) fn play_rumble(
        _config: &MacosGcControllerHapticsConfig,
        _device_ids: &[String],
        effect: &OhMyGamepadRumbleEffectDto,
    ) -> Result<(), HapticsProviderError> {
        let intensity = normalized_intensity(effect);
        if intensity <= 0.0 {
            return Ok(());
        }

        let Some(controller) = preferred_controller() else {
            return Err(HapticsProviderError::Unsupported);
        };
        let Some(haptics) = (unsafe { controller.haptics() }) else {
            return Err(HapticsProviderError::Unsupported);
        };
        if !supports_gc_haptics_engine(&haptics) {
            return Err(HapticsProviderError::Unsupported);
        }

        let engine =
            unsafe { create_default_engine(&haptics) }.ok_or(HapticsProviderError::Unsupported)?;
        unsafe { engine.startAndReturnError() }
            .map_err(|_| HapticsProviderError::TransportClosed)?;

        let pattern = build_basic_rumble_pattern(effect)?;
        let player = unsafe { engine.createPlayerWithPattern_error(&pattern) }
            .map_err(|_| HapticsProviderError::TransportClosed)?;
        unsafe { player.startAtTime_error(CHHapticTimeImmediate) }
            .map_err(|_| HapticsProviderError::TransportClosed)
    }

    pub(super) fn stop_rumble(
        _config: &MacosGcControllerHapticsConfig,
        _device_ids: &[String],
    ) -> Result<(), HapticsProviderError> {
        // 当前最小实现使用固定时长 pattern，不做 engine/player 常驻缓存。
        // 因此 stop 先退化成 no-op，后续接入 registry + player cache 后再补真停振。
        Ok(())
    }

    fn preferred_controller() -> Option<Retained<GCController>> {
        let current = unsafe { GCController::current() };
        if let Some(controller) = current {
            return Some(controller);
        }

        let controllers = unsafe { GCController::controllers() };
        controllers.firstObject()
    }

    fn supports_gc_haptics_engine(haptics: &GCDeviceHaptics) -> bool {
        unsafe { msg_send![haptics, respondsToSelector: sel!(createEngineWithLocality:)] }
    }

    unsafe fn create_default_engine(haptics: &GCDeviceHaptics) -> Option<Retained<CHHapticEngine>> {
        unsafe { msg_send![haptics, createEngineWithLocality: GCHapticsLocalityDefault] }
    }

    fn build_basic_rumble_pattern(
        effect: &OhMyGamepadRumbleEffectDto,
    ) -> Result<Retained<CHHapticPattern>, HapticsProviderError> {
        let intensity_parameter = unsafe {
            CHHapticEventParameter::initWithParameterID_value(
                CHHapticEventParameter::alloc(),
                CHHapticEventParameterIDHapticIntensity,
                normalized_intensity(effect),
            )
        };
        let sharpness_parameter = unsafe {
            CHHapticEventParameter::initWithParameterID_value(
                CHHapticEventParameter::alloc(),
                CHHapticEventParameterIDHapticSharpness,
                normalized_sharpness(effect),
            )
        };
        let parameters = NSArray::from_retained_slice(&[intensity_parameter, sharpness_parameter]);

        let event = unsafe {
            CHHapticEvent::initWithEventType_parameters_relativeTime_duration(
                CHHapticEvent::alloc(),
                CHHapticEventTypeHapticContinuous,
                &parameters,
                f64::from(effect.start_delay_ms) / 1000.0,
                normalized_duration_seconds(effect),
            )
        };
        let events = NSArray::from_retained_slice(&[event]);
        let dynamic_parameters = NSArray::new();

        unsafe {
            CHHapticPattern::initWithEvents_parameters_error(
                CHHapticPattern::alloc(),
                &events,
                &dynamic_parameters,
            )
        }
        .map_err(|_| HapticsProviderError::TransportClosed)
    }

    fn normalized_intensity(effect: &OhMyGamepadRumbleEffectDto) -> f32 {
        clamp_unit(
            effect
                .strong_magnitude
                .max(effect.weak_magnitude)
                .max(effect.left_trigger)
                .max(effect.right_trigger),
        )
    }

    fn normalized_sharpness(effect: &OhMyGamepadRumbleEffectDto) -> f32 {
        let strong = clamp_unit(effect.strong_magnitude.max(effect.left_trigger));
        let weak = clamp_unit(effect.weak_magnitude.max(effect.right_trigger));
        clamp_unit(0.2 + strong * 0.6 + weak * 0.2)
    }

    fn normalized_duration_seconds(effect: &OhMyGamepadRumbleEffectDto) -> f64 {
        f64::from(effect.duration_ms.max(1)) / 1000.0
    }

    fn clamp_unit(value: f32) -> f32 {
        value.clamp(0.0, 1.0)
    }

    #[cfg(test)]
    mod tests {
        use ohmygamepad_protocol::OhMyGamepadRumbleEffectDto;

        use super::{normalized_duration_seconds, normalized_intensity, normalized_sharpness};

        #[test]
        fn normalized_intensity_uses_strongest_channel() {
            let effect = OhMyGamepadRumbleEffectDto {
                strong_magnitude: 0.3,
                weak_magnitude: 0.6,
                left_trigger: 0.2,
                right_trigger: 0.8,
                ..OhMyGamepadRumbleEffectDto::default()
            };

            assert_eq!(normalized_intensity(&effect), 0.8);
        }

        #[test]
        fn normalized_duration_is_at_least_one_millisecond() {
            let effect = OhMyGamepadRumbleEffectDto {
                duration_ms: 0,
                ..OhMyGamepadRumbleEffectDto::default()
            };

            assert_eq!(normalized_duration_seconds(&effect), 0.001);
        }

        #[test]
        fn normalized_sharpness_stays_within_unit_range() {
            let effect = OhMyGamepadRumbleEffectDto {
                strong_magnitude: 1.0,
                weak_magnitude: 1.0,
                left_trigger: 1.0,
                right_trigger: 1.0,
                ..OhMyGamepadRumbleEffectDto::default()
            };

            assert!(normalized_sharpness(&effect) <= 1.0);
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use ohmygamepad_core::HapticsProviderError;
    use ohmygamepad_protocol::OhMyGamepadRumbleEffectDto;

    use super::MacosGcControllerHapticsConfig;

    pub(super) fn play_rumble(
        _config: &MacosGcControllerHapticsConfig,
        _device_ids: &[String],
        _effect: &OhMyGamepadRumbleEffectDto,
    ) -> Result<(), HapticsProviderError> {
        Err(HapticsProviderError::Unsupported)
    }

    pub(super) fn stop_rumble(
        _config: &MacosGcControllerHapticsConfig,
        _device_ids: &[String],
    ) -> Result<(), HapticsProviderError> {
        Err(HapticsProviderError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::{MacosGcControllerHapticsConfig, MacosGcControllerHapticsProvider};

    #[test]
    fn default_config_prefers_bluetooth_primary_controller() {
        let provider = MacosGcControllerHapticsProvider::default();

        assert_eq!(
            provider.config(),
            &MacosGcControllerHapticsConfig {
                bluetooth_only: true,
                prefer_primary_controller: true,
            }
        );
    }
}
