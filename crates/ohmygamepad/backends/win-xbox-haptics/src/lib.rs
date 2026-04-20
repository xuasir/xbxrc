use ohmygamepad_core::{HapticsProvider, HapticsProviderError};
use ohmygamepad_protocol::OhMyGamepadRumbleEffectDto;

/**
 * Windows 侧优先走 WinRT Gamepad vibration，尽量保留四路语义；
 * WinRT 无法命中时，再退回 XInput 双马达，保证基础震动至少可用。
 */
#[derive(Default)]
pub struct WindowsXboxHapticsProvider;

impl HapticsProvider for WindowsXboxHapticsProvider {
    fn play_rumble(
        &self,
        device_ids: &[String],
        effect: &OhMyGamepadRumbleEffectDto,
    ) -> Result<(), HapticsProviderError> {
        platform::play_rumble(device_ids, effect)
    }

    fn stop_rumble(&self, device_ids: &[String]) -> Result<(), HapticsProviderError> {
        platform::stop_rumble(device_ids)
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::{
        collections::{BTreeSet, HashMap},
        sync::{
            atomic::{AtomicU64, Ordering},
            Mutex, OnceLock,
        },
        thread,
        time::Duration,
    };

    use ohmygamepad_core::HapticsProviderError;
    use ohmygamepad_protocol::OhMyGamepadRumbleEffectDto;
    use windows::{
        core::Result as WinResult,
        Gaming::Input::{Gamepad, GamepadVibration, RawGameController},
        Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED},
    };
    use windows_sys::Win32::UI::Input::XboxController::{XInputSetState, XINPUT_VIBRATION};

    static STOP_TOKENS: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    static NEXT_STOP_TOKEN: AtomicU64 = AtomicU64::new(1);

    pub(super) fn play_rumble(
        device_ids: &[String],
        effect: &OhMyGamepadRumbleEffectDto,
    ) -> Result<(), HapticsProviderError> {
        let targets = resolve_targets(device_ids);
        if targets.is_empty() {
            return Err(HapticsProviderError::Unsupported);
        }

        apply_effect(&targets, effect)?;
        schedule_stop(device_ids, effect.duration_ms);
        Ok(())
    }

    pub(super) fn stop_rumble(device_ids: &[String]) -> Result<(), HapticsProviderError> {
        bump_stop_tokens(device_ids);
        apply_stop(&resolve_targets(device_ids))
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum ResolvedTarget {
        WinRt { device_id: String, index: u32 },
        XInput { device_id: String, user: u32 },
    }

    fn apply_effect(
        targets: &[ResolvedTarget],
        effect: &OhMyGamepadRumbleEffectDto,
    ) -> Result<(), HapticsProviderError> {
        let mut applied = false;
        let mut transport_failed = false;

        for target in targets {
            match target {
                ResolvedTarget::WinRt { index, .. } => {
                    match apply_winrt_vibration(*index, effect) {
                        Ok(true) => applied = true,
                        Ok(false) => {}
                        Err(_) => transport_failed = true,
                    }
                }
                ResolvedTarget::XInput { user, .. } => {
                    match apply_xinput_vibration(*user, effect) {
                        Ok(true) => applied = true,
                        Ok(false) => {}
                        Err(_) => transport_failed = true,
                    }
                }
            }
        }

        if applied {
            Ok(())
        } else if transport_failed {
            Err(HapticsProviderError::TransportClosed)
        } else {
            Err(HapticsProviderError::Unsupported)
        }
    }

    fn apply_stop(targets: &[ResolvedTarget]) -> Result<(), HapticsProviderError> {
        let mut applied = false;
        let mut transport_failed = false;

        for target in targets {
            match target {
                ResolvedTarget::WinRt { index, .. } => match apply_winrt_stop(*index) {
                    Ok(true) => applied = true,
                    Ok(false) => {}
                    Err(_) => transport_failed = true,
                },
                ResolvedTarget::XInput { user, .. } => match apply_xinput_stop(*user) {
                    Ok(true) => applied = true,
                    Ok(false) => {}
                    Err(_) => transport_failed = true,
                },
            }
        }

        if applied {
            Ok(())
        } else if transport_failed {
            Err(HapticsProviderError::TransportClosed)
        } else {
            Err(HapticsProviderError::Unsupported)
        }
    }

    fn resolve_targets(device_ids: &[String]) -> Vec<ResolvedTarget> {
        let mut targets = Vec::new();
        let mut seen = BTreeSet::new();

        for device_id in device_ids {
            if let Some(index) = parse_device_index(device_id) {
                if winrt_gamepad_exists(index) {
                    let winrt_key = format!("winrt:{index}");
                    if seen.insert(winrt_key) {
                        targets.push(ResolvedTarget::WinRt {
                            device_id: device_id.clone(),
                            index,
                        });
                    }
                    continue;
                }
            }

            if let Some(user) = parse_xinput_user_index(device_id) {
                let xinput_key = format!("xinput:{user}");
                if seen.insert(xinput_key) {
                    targets.push(ResolvedTarget::XInput {
                        device_id: device_id.clone(),
                        user,
                    });
                }
            }
        }

        targets
    }

    fn apply_winrt_vibration(index: u32, effect: &OhMyGamepadRumbleEffectDto) -> WinResult<bool> {
        let Some(gamepad) = resolve_winrt_gamepad(index)? else {
            return Ok(false);
        };

        let vibration = GamepadVibration {
            LeftMotor: normalized_component(effect.strong_magnitude),
            RightMotor: normalized_component(effect.weak_magnitude),
            LeftTrigger: normalized_component(effect.left_trigger),
            RightTrigger: normalized_component(effect.right_trigger),
        };
        gamepad.SetVibration(vibration)?;
        Ok(true)
    }

    fn apply_winrt_stop(index: u32) -> WinResult<bool> {
        let Some(gamepad) = resolve_winrt_gamepad(index)? else {
            return Ok(false);
        };

        gamepad.SetVibration(GamepadVibration {
            LeftMotor: 0.0,
            RightMotor: 0.0,
            LeftTrigger: 0.0,
            RightTrigger: 0.0,
        })?;
        Ok(true)
    }

    fn resolve_winrt_gamepad(index: u32) -> WinResult<Option<Gamepad>> {
        // WinRT 调用前确保当前线程进入 MTA；重复调用时返回值可安全忽略。
        let _ = unsafe { CoInitializeEx(Some(std::ptr::null()), COINIT_MULTITHREADED) };

        let raw_controllers = RawGameController::RawGameControllers()?;
        if index < raw_controllers.Size()? {
            let controller = raw_controllers.GetAt(index)?;
            if let Ok(gamepad) = Gamepad::FromGameController(&controller) {
                return Ok(Some(gamepad));
            }
        }

        let gamepads = Gamepad::Gamepads()?;
        if index < gamepads.Size()? {
            return Ok(Some(gamepads.GetAt(index)?));
        }

        Ok(None)
    }

    fn winrt_gamepad_exists(index: u32) -> bool {
        resolve_winrt_gamepad(index)
            .map(|gamepad| gamepad.is_some())
            .unwrap_or(false)
    }

    fn apply_xinput_vibration(user: u32, effect: &OhMyGamepadRumbleEffectDto) -> Result<bool, ()> {
        apply_xinput_state(
            user,
            XINPUT_VIBRATION {
                wLeftMotorSpeed: magnitude_to_motor_speed(effect.strong_magnitude),
                wRightMotorSpeed: magnitude_to_motor_speed(effect.weak_magnitude),
            },
        )
    }

    fn apply_xinput_stop(user: u32) -> Result<bool, ()> {
        apply_xinput_state(
            user,
            XINPUT_VIBRATION {
                wLeftMotorSpeed: 0,
                wRightMotorSpeed: 0,
            },
        )
    }

    fn apply_xinput_state(user: u32, vibration: XINPUT_VIBRATION) -> Result<bool, ()> {
        let result = unsafe { XInputSetState(user, &vibration) };
        match result {
            0 => Ok(true),
            1167 => Ok(false),
            _ => Err(()),
        }
    }

    fn schedule_stop(device_ids: &[String], duration_ms: u16) {
        if duration_ms == 0 {
            return;
        }

        let tracked_device_ids = device_ids.to_vec();
        let token = NEXT_STOP_TOKEN.fetch_add(1, Ordering::Relaxed);
        register_stop_tokens(&tracked_device_ids, token);

        thread::spawn(move || {
            thread::sleep(Duration::from_millis(u64::from(duration_ms)));

            let stoppable_ids = tracked_device_ids
                .iter()
                .filter(|device_id| matches_stop_token(device_id, token))
                .cloned()
                .collect::<Vec<_>>();

            if stoppable_ids.is_empty() {
                return;
            }

            let _ = apply_stop(&resolve_targets(&stoppable_ids));
        });
    }

    fn register_stop_tokens(device_ids: &[String], token: u64) {
        let mut guard = stop_tokens().lock().expect("lock stop tokens");
        for device_id in device_ids {
            guard.insert(device_id.clone(), token);
        }
    }

    fn bump_stop_tokens(device_ids: &[String]) {
        let token = NEXT_STOP_TOKEN.fetch_add(1, Ordering::Relaxed);
        register_stop_tokens(device_ids, token);
    }

    fn matches_stop_token(device_id: &str, token: u64) -> bool {
        stop_tokens()
            .lock()
            .expect("lock stop tokens")
            .get(device_id)
            .copied()
            == Some(token)
    }

    fn stop_tokens() -> &'static Mutex<HashMap<String, u64>> {
        STOP_TOKENS.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn normalized_component(magnitude: f32) -> f64 {
        f64::from(magnitude.clamp(0.0, 1.0))
    }

    fn magnitude_to_motor_speed(magnitude: f32) -> u16 {
        let normalized = magnitude.clamp(0.0, 1.0);
        (normalized * u16::MAX as f32).round() as u16
    }

    fn parse_device_index(device_id: &str) -> Option<u32> {
        let digits = trailing_ascii_digits(device_id)?;
        digits.parse::<u32>().ok()
    }

    fn parse_xinput_user_index(device_id: &str) -> Option<u32> {
        let user = parse_device_index(device_id)?;
        (user <= 3).then_some(user)
    }

    fn trailing_ascii_digits(device_id: &str) -> Option<&str> {
        device_id
            .rsplit(|ch: char| !ch.is_ascii_digit())
            .find(|segment| !segment.is_empty())
    }

    #[cfg(test)]
    mod tests {
        use super::{
            magnitude_to_motor_speed, normalized_component, parse_device_index,
            parse_xinput_user_index,
        };

        #[test]
        fn parses_numeric_xinput_suffixes() {
            assert_eq!(parse_xinput_user_index("0"), Some(0));
            assert_eq!(parse_xinput_user_index("gilrs:1"), Some(1));
            assert_eq!(parse_xinput_user_index("xinput-user-3"), Some(3));
            assert_eq!(parse_xinput_user_index("4"), None);
            assert_eq!(parse_xinput_user_index("keyboard"), None);
        }

        #[test]
        fn parses_numeric_device_suffixes_for_winrt_lookup() {
            assert_eq!(parse_device_index("0"), Some(0));
            assert_eq!(parse_device_index("gilrs:2"), Some(2));
            assert_eq!(parse_device_index("raw-controller-11"), Some(11));
            assert_eq!(parse_device_index("keyboard"), None);
        }

        #[test]
        fn maps_normalized_magnitude_to_xinput_motor_range() {
            assert_eq!(magnitude_to_motor_speed(-1.0), 0);
            assert_eq!(magnitude_to_motor_speed(0.0), 0);
            assert_eq!(magnitude_to_motor_speed(1.0), u16::MAX);
            assert!(magnitude_to_motor_speed(0.5) >= 32767);
        }

        #[test]
        fn clamps_winrt_components_into_normalized_range() {
            assert_eq!(normalized_component(-0.1), 0.0);
            assert_eq!(normalized_component(0.25), 0.25);
            assert_eq!(normalized_component(1.2), 1.0);
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use ohmygamepad_core::HapticsProviderError;
    use ohmygamepad_protocol::OhMyGamepadRumbleEffectDto;

    pub(super) fn play_rumble(
        _device_ids: &[String],
        _effect: &OhMyGamepadRumbleEffectDto,
    ) -> Result<(), HapticsProviderError> {
        Err(HapticsProviderError::Unsupported)
    }

    pub(super) fn stop_rumble(_device_ids: &[String]) -> Result<(), HapticsProviderError> {
        Err(HapticsProviderError::Unsupported)
    }
}
