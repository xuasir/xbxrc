use crate::InputCoreConfig;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DesktopInputProviderKind {
    Sdl3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DesktopHapticsProviderKind {
    Sdl3Gamepad,
    WinXboxHaptics,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SelectedDesktopRuntimeProviders {
    pub input_provider: DesktopInputProviderKind,
    pub haptics_provider: DesktopHapticsProviderKind,
}

/**
 * 先把“输入 provider / haptics provider 怎么选”从 facade 抽回 core。
 * 当前策略还很简单，但后续加 DualSense / Windows haptics 时可以继续扩展这里。
 */
pub struct DesktopDriverSelector;

impl DesktopDriverSelector {
    pub fn select(_config: &InputCoreConfig) -> SelectedDesktopRuntimeProviders {
        #[cfg(target_os = "windows")]
        let haptics_provider = DesktopHapticsProviderKind::WinXboxHaptics;
        #[cfg(not(target_os = "windows"))]
        let haptics_provider = DesktopHapticsProviderKind::Sdl3Gamepad;

        SelectedDesktopRuntimeProviders {
            input_provider: DesktopInputProviderKind::Sdl3,
            haptics_provider,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        DesktopDriverSelector, DesktopHapticsProviderKind, DesktopInputProviderKind,
        InputCoreConfig,
    };

    #[test]
    fn selector_chooses_expected_default_haptics_provider() {
        let selection = DesktopDriverSelector::select(&InputCoreConfig::default());

        assert_eq!(selection.input_provider, DesktopInputProviderKind::Sdl3);
        #[cfg(target_os = "windows")]
        assert_eq!(
            selection.haptics_provider,
            DesktopHapticsProviderKind::WinXboxHaptics
        );
        #[cfg(not(target_os = "windows"))]
        assert_eq!(
            selection.haptics_provider,
            DesktopHapticsProviderKind::Sdl3Gamepad
        );
    }
}
