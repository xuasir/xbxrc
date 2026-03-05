#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OhMyGamepadStreamPushModeDto {
    #[default]
    OnChange,
    FixedRate,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OhMyGamepadSamplingPresetDto {
    PowerSaver,
    #[default]
    Balanced,
    HighResponse,
    MaxPrecision,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OhMyGamepadSamplingConfigDto {
    pub backend_poll_rate_hz: u16,
    pub logical_pad_sample_rate_hz: u16,
    pub ui_push_rate_hz: u16,
    pub stream_push_mode: OhMyGamepadStreamPushModeDto,
    pub stream_push_rate_hz: Option<u16>,
}

impl OhMyGamepadSamplingConfigDto {
    pub fn from_preset(preset: OhMyGamepadSamplingPresetDto) -> Self {
        match preset {
            OhMyGamepadSamplingPresetDto::PowerSaver => Self {
                backend_poll_rate_hz: 120,
                logical_pad_sample_rate_hz: 120,
                ui_push_rate_hz: 30,
                stream_push_mode: OhMyGamepadStreamPushModeDto::OnChange,
                stream_push_rate_hz: None,
            },
            OhMyGamepadSamplingPresetDto::Balanced => Self {
                backend_poll_rate_hz: 250,
                logical_pad_sample_rate_hz: 250,
                ui_push_rate_hz: 60,
                stream_push_mode: OhMyGamepadStreamPushModeDto::OnChange,
                stream_push_rate_hz: None,
            },
            OhMyGamepadSamplingPresetDto::HighResponse => Self {
                backend_poll_rate_hz: 500,
                logical_pad_sample_rate_hz: 250,
                ui_push_rate_hz: 60,
                stream_push_mode: OhMyGamepadStreamPushModeDto::OnChange,
                stream_push_rate_hz: None,
            },
            OhMyGamepadSamplingPresetDto::MaxPrecision => Self {
                backend_poll_rate_hz: 1000,
                logical_pad_sample_rate_hz: 500,
                ui_push_rate_hz: 120,
                stream_push_mode: OhMyGamepadStreamPushModeDto::OnChange,
                stream_push_rate_hz: None,
            },
        }
    }
}

impl Default for OhMyGamepadSamplingConfigDto {
    fn default() -> Self {
        Self::from_preset(OhMyGamepadSamplingPresetDto::Balanced)
    }
}

#[cfg(test)]
mod tests {
    use super::{OhMyGamepadSamplingConfigDto, OhMyGamepadSamplingPresetDto};

    #[test]
    fn balanced_preset_matches_default_sampling() {
        assert_eq!(
            OhMyGamepadSamplingConfigDto::default(),
            OhMyGamepadSamplingConfigDto::from_preset(OhMyGamepadSamplingPresetDto::Balanced)
        );
    }

    #[test]
    fn max_precision_preset_increases_polling_rates() {
        let config =
            OhMyGamepadSamplingConfigDto::from_preset(OhMyGamepadSamplingPresetDto::MaxPrecision);

        assert_eq!(config.backend_poll_rate_hz, 1000);
        assert_eq!(config.logical_pad_sample_rate_hz, 500);
        assert_eq!(config.ui_push_rate_hz, 120);
    }
}
