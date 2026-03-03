#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum GamepadStreamPushModeDto {
    #[default]
    OnChange,
    FixedRate,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GamepadSamplingConfigDto {
    pub backend_poll_rate_hz: u16,
    pub logical_pad_sample_rate_hz: u16,
    pub ui_push_rate_hz: u16,
    pub stream_push_mode: GamepadStreamPushModeDto,
    pub stream_push_rate_hz: Option<u16>,
}

impl Default for GamepadSamplingConfigDto {
    fn default() -> Self {
        Self {
            backend_poll_rate_hz: 250,
            logical_pad_sample_rate_hz: 250,
            ui_push_rate_hz: 60,
            stream_push_mode: GamepadStreamPushModeDto::OnChange,
            stream_push_rate_hz: None,
        }
    }
}
