use crate::InputCoreConfig;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DesktopInputProviderKind {
    Gilrs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DesktopHapticsProviderKind {
    GilrsBasic,
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
        SelectedDesktopRuntimeProviders {
            input_provider: DesktopInputProviderKind::Gilrs,
            haptics_provider: DesktopHapticsProviderKind::GilrsBasic,
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
    fn selector_chooses_gilrs_and_basic_haptics_by_default() {
        let selection = DesktopDriverSelector::select(&InputCoreConfig::default());

        assert_eq!(selection.input_provider, DesktopInputProviderKind::Gilrs);
        assert_eq!(
            selection.haptics_provider,
            DesktopHapticsProviderKind::GilrsBasic
        );
    }
}
