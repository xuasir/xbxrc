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
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::sync::Once;
    use std::time::{Duration, Instant};

    use objc2::{msg_send, rc::Retained, runtime::ProtocolObject, sel, AnyThread};
    use objc2_core_haptics::{
        CHHapticEngine, CHHapticEvent, CHHapticEventParameter,
        CHHapticEventParameterIDHapticIntensity, CHHapticEventParameterIDHapticSharpness,
        CHHapticEventTypeHapticContinuous, CHHapticPattern, CHHapticPatternPlayer,
        CHHapticTimeImmediate,
    };
    use objc2_foundation::NSArray;
    use objc2_game_controller::{
        GCController, GCDevice, GCDeviceHaptics, GCHapticsLocality, GCHapticsLocalityDefault,
        GCHapticsLocalityHandles, GCHapticsLocalityLeftHandle, GCHapticsLocalityLeftTrigger,
        GCHapticsLocalityRightHandle, GCHapticsLocalityRightTrigger, GCHapticsLocalityTriggers,
    };
    use ohmygamepad_core::HapticsProviderError;
    use ohmygamepad_protocol::OhMyGamepadRumbleEffectDto;

    use super::MacosGcControllerHapticsConfig;

    static LOGGED_HAPTICS_CAPABILITIES: Once = Once::new();
    const LOCALITY_MERGE_WINDOW_MS: u64 = 8;
    const HANDLE_MIN_DURATION_MS: u16 = 16;
    const TRIGGER_MIN_DURATION_MS: u16 = 12;
    const LOCALITY_DURATION_TRIM_MS: u16 = 8;
    const LOCALITY_REPLACE_EPSILON: f32 = 0.08;
    const COMBINED_INTENSITY_CEILING: f32 = 0.72;
    const HANDLE_INTENSITY_CEILING: f32 = 0.74;
    const TRIGGER_INTENSITY_CEILING: f32 = 0.64;
    thread_local! {
        static LOCALITY_PLAYBACK_CACHE: RefCell<HashMap<String, LocalityPlaybackState>> = RefCell::new(HashMap::new());
    }

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
        log_haptics_capabilities_once(&controller, &haptics);
        if !supports_gc_haptics_engine(&haptics) {
            return Err(HapticsProviderError::Unsupported);
        }

        let supported_localities = unsafe { haptics.supportedLocalities() };
        let mut dispatched = false;

        // Xbox rumble 的四路语义应该尽量保留到 macOS locality，而不是继续压回 Default。
        dispatched |= dispatch_channel(
            &haptics,
            &supported_localities,
            locality_left_handle(),
            LocalityProfileKind::HandleLeft,
            effect.strong_magnitude,
            effect,
        )?;
        dispatched |= dispatch_channel(
            &haptics,
            &supported_localities,
            locality_right_handle(),
            LocalityProfileKind::HandleRight,
            effect.weak_magnitude,
            effect,
        )?;
        dispatched |= dispatch_channel(
            &haptics,
            &supported_localities,
            locality_left_trigger(),
            LocalityProfileKind::TriggerLeft,
            effect.left_trigger,
            effect,
        )?;
        dispatched |= dispatch_channel(
            &haptics,
            &supported_localities,
            locality_right_trigger(),
            LocalityProfileKind::TriggerRight,
            effect.right_trigger,
            effect,
        )?;

        if dispatched {
            return Ok(());
        }

        if let Some(locality) = resolve_fallback_locality(&supported_localities, effect) {
            return dispatch_locality(&haptics, locality, localized_effect_from_combined(effect))
                .map(|_| ());
        }

        Err(HapticsProviderError::Unsupported)
    }

    pub(super) fn stop_rumble(
        _config: &MacosGcControllerHapticsConfig,
        _device_ids: &[String],
    ) -> Result<(), HapticsProviderError> {
        LOCALITY_PLAYBACK_CACHE.with(|cache| {
            for state in cache.borrow_mut().values_mut() {
                let _ = stop_locality_player(state);
            }
        });
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

    fn log_haptics_capabilities_once(controller: &GCController, haptics: &GCDeviceHaptics) {
        LOGGED_HAPTICS_CAPABILITIES.call_once(|| {
            let controller_name = unsafe { controller.vendorName() }
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let product_category = unsafe { controller.productCategory() }.to_string();
            let mut localities = unsafe { haptics.supportedLocalities() }
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>();
            localities.sort();

            log::info!(
                "[ohmygamepad][macos-haptics] controller={} product_category={} supported_localities={:?}",
                controller_name,
                product_category,
                localities
            );
        });
    }

    fn dispatch_channel(
        haptics: &GCDeviceHaptics,
        supported_localities: &objc2_foundation::NSSet<GCHapticsLocality>,
        locality: &GCHapticsLocality,
        profile: LocalityProfileKind,
        magnitude: f32,
        effect: &OhMyGamepadRumbleEffectDto,
    ) -> Result<bool, HapticsProviderError> {
        let localized_effect = localized_effect_for_profile(profile, magnitude, effect.duration_ms);
        if localized_effect.intensity <= 0.0 {
            return Ok(false);
        }
        if !supports_locality(supported_localities, locality) {
            return Ok(false);
        }

        dispatch_locality(haptics, locality, localized_effect)?;
        Ok(true)
    }

    fn dispatch_locality(
        haptics: &GCDeviceHaptics,
        locality: &GCHapticsLocality,
        localized_effect: LocalizedRumbleEffect,
    ) -> Result<(), HapticsProviderError> {
        let locality_key = locality.to_string();
        LOCALITY_PLAYBACK_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            let state = cache
                .entry(locality_key)
                .or_insert_with(|| LocalityPlaybackState {
                    engine: None,
                    player: None,
                    last_effect: None,
                    last_started_at: None,
                });

            let engine = ensure_locality_engine(haptics, locality, state)?;
            if should_keep_current_player(state, localized_effect) {
                return Ok(());
            }
            let _ = stop_locality_player(state);

            let pattern = build_basic_rumble_pattern(localized_effect)?;
            let player = unsafe { engine.createPlayerWithPattern_error(&pattern) }
                .map_err(|_| HapticsProviderError::TransportClosed)?;
            unsafe { player.startAtTime_error(CHHapticTimeImmediate) }
                .map_err(|_| HapticsProviderError::TransportClosed)?;
            state.player = Some(player);
            state.last_effect = Some(localized_effect);
            state.last_started_at = Some(Instant::now());
            Ok(())
        })
    }

    fn resolve_fallback_locality(
        supported_localities: &objc2_foundation::NSSet<GCHapticsLocality>,
        effect: &OhMyGamepadRumbleEffectDto,
    ) -> Option<&'static GCHapticsLocality> {
        if (effect.strong_magnitude > 0.0 || effect.weak_magnitude > 0.0)
            && supports_locality(supported_localities, locality_handles())
        {
            return Some(locality_handles());
        }

        if (effect.left_trigger > 0.0 || effect.right_trigger > 0.0)
            && supports_locality(supported_localities, locality_triggers())
        {
            return Some(locality_triggers());
        }

        if supports_locality(supported_localities, locality_default()) {
            return Some(locality_default());
        }

        None
    }

    fn supports_locality(
        supported_localities: &objc2_foundation::NSSet<GCHapticsLocality>,
        locality: &GCHapticsLocality,
    ) -> bool {
        supported_localities
            .iter()
            .any(|value| value.to_string() == locality.to_string())
    }

    unsafe fn create_engine(
        haptics: &GCDeviceHaptics,
        locality: &GCHapticsLocality,
    ) -> Option<Retained<CHHapticEngine>> {
        unsafe { msg_send![haptics, createEngineWithLocality: locality] }
    }

    fn locality_default() -> &'static GCHapticsLocality {
        unsafe { GCHapticsLocalityDefault }
    }

    fn locality_handles() -> &'static GCHapticsLocality {
        unsafe { GCHapticsLocalityHandles }
    }

    fn locality_left_handle() -> &'static GCHapticsLocality {
        unsafe { GCHapticsLocalityLeftHandle }
    }

    fn locality_right_handle() -> &'static GCHapticsLocality {
        unsafe { GCHapticsLocalityRightHandle }
    }

    fn locality_triggers() -> &'static GCHapticsLocality {
        unsafe { GCHapticsLocalityTriggers }
    }

    fn locality_left_trigger() -> &'static GCHapticsLocality {
        unsafe { GCHapticsLocalityLeftTrigger }
    }

    fn locality_right_trigger() -> &'static GCHapticsLocality {
        unsafe { GCHapticsLocalityRightTrigger }
    }

    fn build_basic_rumble_pattern(
        localized_effect: LocalizedRumbleEffect,
    ) -> Result<Retained<CHHapticPattern>, HapticsProviderError> {
        let intensity_parameter = unsafe {
            CHHapticEventParameter::initWithParameterID_value(
                CHHapticEventParameter::alloc(),
                CHHapticEventParameterIDHapticIntensity,
                localized_effect.intensity,
            )
        };
        let sharpness_parameter = unsafe {
            CHHapticEventParameter::initWithParameterID_value(
                CHHapticEventParameter::alloc(),
                CHHapticEventParameterIDHapticSharpness,
                localized_effect.sharpness,
            )
        };
        let parameters = NSArray::from_retained_slice(&[intensity_parameter, sharpness_parameter]);

        let event = unsafe {
            CHHapticEvent::initWithEventType_parameters_relativeTime_duration(
                CHHapticEvent::alloc(),
                CHHapticEventTypeHapticContinuous,
                &parameters,
                0.0,
                normalized_duration_seconds(localized_effect.duration_ms),
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

    #[derive(Clone, Copy)]
    struct LocalizedRumbleEffect {
        intensity: f32,
        sharpness: f32,
        duration_ms: u16,
    }

    #[derive(Clone, Copy)]
    enum LocalityProfileKind {
        HandleLeft,
        HandleRight,
        TriggerLeft,
        TriggerRight,
    }

    struct LocalityPlaybackState {
        engine: Option<Retained<CHHapticEngine>>,
        player: Option<Retained<ProtocolObject<dyn CHHapticPatternPlayer>>>,
        last_effect: Option<LocalizedRumbleEffect>,
        last_started_at: Option<Instant>,
    }

    fn ensure_locality_engine(
        haptics: &GCDeviceHaptics,
        locality: &GCHapticsLocality,
        state: &mut LocalityPlaybackState,
    ) -> Result<Retained<CHHapticEngine>, HapticsProviderError> {
        if let Some(engine) = state.engine.as_ref() {
            unsafe { engine.startAndReturnError() }
                .map_err(|_| HapticsProviderError::TransportClosed)?;
            return Ok(engine.clone());
        }

        let engine =
            unsafe { create_engine(haptics, locality) }.ok_or(HapticsProviderError::Unsupported)?;
        unsafe { engine.setPlaysHapticsOnly(true) };
        unsafe { engine.startAndReturnError() }
            .map_err(|_| HapticsProviderError::TransportClosed)?;
        state.engine = Some(engine.clone());
        Ok(engine)
    }

    fn stop_locality_player(state: &mut LocalityPlaybackState) -> Result<(), HapticsProviderError> {
        let Some(player) = state.player.take() else {
            return Ok(());
        };
        state.last_effect = None;
        state.last_started_at = None;

        unsafe { player.stopAtTime_error(CHHapticTimeImmediate) }
            .or_else(|_| unsafe { player.cancelAndReturnError() })
            .map_err(|_| HapticsProviderError::TransportClosed)
    }

    fn should_keep_current_player(
        state: &LocalityPlaybackState,
        next_effect: LocalizedRumbleEffect,
    ) -> bool {
        let Some(previous_effect) = state.last_effect else {
            return false;
        };
        let Some(last_started_at) = state.last_started_at else {
            return false;
        };
        if last_started_at.elapsed() > Duration::from_millis(LOCALITY_MERGE_WINDOW_MS) {
            return false;
        }

        next_effect.intensity <= previous_effect.intensity + LOCALITY_REPLACE_EPSILON
            && next_effect.sharpness <= previous_effect.sharpness + LOCALITY_REPLACE_EPSILON
    }

    fn localized_effect_from_combined(
        effect: &OhMyGamepadRumbleEffectDto,
    ) -> LocalizedRumbleEffect {
        LocalizedRumbleEffect {
            intensity: normalized_intensity(effect),
            sharpness: normalized_sharpness(effect),
            duration_ms: normalized_duration_ms(effect.duration_ms, false),
        }
    }

    fn localized_effect_for_profile(
        profile: LocalityProfileKind,
        magnitude: f32,
        duration_ms: u16,
    ) -> LocalizedRumbleEffect {
        let normalized_magnitude = clamp_unit(magnitude);
        let is_trigger = matches!(
            profile,
            LocalityProfileKind::TriggerLeft | LocalityProfileKind::TriggerRight
        );
        let intensity = if is_trigger {
            clamp_unit(normalized_magnitude.powf(1.18) * TRIGGER_INTENSITY_CEILING)
        } else {
            clamp_unit(normalized_magnitude.powf(1.28) * HANDLE_INTENSITY_CEILING)
        };
        let sharpness = match profile {
            LocalityProfileKind::HandleLeft => localized_sharpness(intensity, 0.62),
            LocalityProfileKind::HandleRight => localized_sharpness(intensity, 0.24),
            LocalityProfileKind::TriggerLeft | LocalityProfileKind::TriggerRight => {
                localized_sharpness(intensity, 0.68)
            }
        };

        LocalizedRumbleEffect {
            intensity,
            sharpness,
            duration_ms: normalized_duration_ms(duration_ms, is_trigger),
        }
    }

    fn normalized_intensity(effect: &OhMyGamepadRumbleEffectDto) -> f32 {
        let peak = effect
            .strong_magnitude
            .max(effect.weak_magnitude)
            .max(effect.left_trigger)
            .max(effect.right_trigger);
        clamp_unit(clamp_unit(peak).powf(1.18) * COMBINED_INTENSITY_CEILING)
    }

    fn normalized_sharpness(effect: &OhMyGamepadRumbleEffectDto) -> f32 {
        let strong = clamp_unit(effect.strong_magnitude.max(effect.left_trigger));
        let weak = clamp_unit(effect.weak_magnitude.max(effect.right_trigger));
        clamp_unit(0.12 + strong * 0.42 + weak * 0.16)
    }

    fn localized_sharpness(magnitude: f32, bias: f32) -> f32 {
        clamp_unit(0.08 + clamp_unit(magnitude) * 0.24 + bias * 0.36)
    }

    fn normalized_duration_ms(duration_ms: u16, trigger_preferred: bool) -> u16 {
        let min_duration_ms = if trigger_preferred {
            TRIGGER_MIN_DURATION_MS
        } else {
            HANDLE_MIN_DURATION_MS
        };
        duration_ms
            .saturating_sub(LOCALITY_DURATION_TRIM_MS)
            .max(min_duration_ms)
    }

    fn normalized_duration_seconds(duration_ms: u16) -> f64 {
        f64::from(duration_ms.max(1)) / 1000.0
    }

    fn clamp_unit(value: f32) -> f32 {
        value.clamp(0.0, 1.0)
    }

    #[cfg(test)]
    mod tests {
        use ohmygamepad_protocol::OhMyGamepadRumbleEffectDto;

        use super::{
            localized_effect_for_profile, localized_effect_from_combined, localized_sharpness,
            normalized_duration_ms, normalized_duration_seconds, normalized_intensity,
            normalized_sharpness, LocalityProfileKind, HANDLE_MIN_DURATION_MS,
            TRIGGER_MIN_DURATION_MS,
        };

        #[test]
        fn normalized_intensity_uses_strongest_channel() {
            let effect = OhMyGamepadRumbleEffectDto {
                strong_magnitude: 0.3,
                weak_magnitude: 0.6,
                left_trigger: 0.2,
                right_trigger: 0.8,
                ..OhMyGamepadRumbleEffectDto::default()
            };

            let normalized = normalized_intensity(&effect);
            assert!(normalized > 0.0);
            assert!(normalized < 0.8);
        }

        #[test]
        fn normalized_duration_is_at_least_one_millisecond() {
            let effect = OhMyGamepadRumbleEffectDto {
                duration_ms: 0,
                ..OhMyGamepadRumbleEffectDto::default()
            };

            assert_eq!(
                normalized_duration_seconds(normalized_duration_ms(effect.duration_ms, false)),
                f64::from(HANDLE_MIN_DURATION_MS) / 1000.0
            );
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

        #[test]
        fn localized_sharpness_stays_within_unit_range() {
            assert!(localized_sharpness(1.0, 1.0) <= 1.0);
            assert!(localized_sharpness(0.0, 0.0) >= 0.0);
        }

        #[test]
        fn combined_localized_effect_matches_previous_normalization() {
            let effect = OhMyGamepadRumbleEffectDto {
                strong_magnitude: 0.7,
                weak_magnitude: 0.3,
                left_trigger: 0.2,
                right_trigger: 0.1,
                ..OhMyGamepadRumbleEffectDto::default()
            };

            let localized = localized_effect_from_combined(&effect);
            assert_eq!(localized.intensity, normalized_intensity(&effect));
            assert_eq!(localized.sharpness, normalized_sharpness(&effect));
        }

        #[test]
        fn trigger_profile_keeps_shorter_min_duration_and_higher_sharpness() {
            let localized = localized_effect_for_profile(LocalityProfileKind::TriggerLeft, 0.5, 0);
            assert_eq!(localized.duration_ms, TRIGGER_MIN_DURATION_MS);
            assert!(localized.sharpness > 0.3);
        }

        #[test]
        fn handle_profile_stays_below_raw_peak_for_realistic_default() {
            let localized = localized_effect_for_profile(LocalityProfileKind::HandleLeft, 1.0, 32);
            assert!(localized.intensity < 1.0);
            assert!(localized.sharpness < 0.8);
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
