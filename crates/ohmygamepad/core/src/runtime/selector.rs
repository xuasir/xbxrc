use crate::InputCoreConfig;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DesktopInputProviderKind {
    Gilrs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DesktopHapticsProviderKind {
    GilrsBasic,
    MacosGcController,
    WindowsXbox,
    None,
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
        #[cfg(target_os = "macos")]
        let haptics_provider = DesktopHapticsProviderKind::MacosGcController;
        #[cfg(target_os = "windows")]
        let haptics_provider = DesktopHapticsProviderKind::WindowsXbox;
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let haptics_provider = DesktopHapticsProviderKind::GilrsBasic;

        SelectedDesktopRuntimeProviders {
            input_provider: DesktopInputProviderKind::Gilrs,
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

        assert_eq!(selection.input_provider, DesktopInputProviderKind::Gilrs);
        #[cfg(target_os = "macos")]
        assert_eq!(
            selection.haptics_provider,
            DesktopHapticsProviderKind::MacosGcController
        );
        #[cfg(target_os = "windows")]
        assert_eq!(
            selection.haptics_provider,
            DesktopHapticsProviderKind::WindowsXbox
        );
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        assert_eq!(
            selection.haptics_provider,
            DesktopHapticsProviderKind::GilrsBasic
        );
    }
}
