use ohmygamepad_protocol::OhMyGamepadRumbleEffectDto;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HapticsProviderError {
    Unsupported,
    TransportClosed,
}

/**
 * 高级/基础震动统一收口到这一层。
 * 现阶段 gilrs/mock service 还没有完全下沉到 core trait，但 bridge/facade 已能开始围绕它做选择。
 */
pub trait HapticsProvider: Send {
    fn play_rumble(
        &self,
        device_ids: &[String],
        effect: &OhMyGamepadRumbleEffectDto,
    ) -> Result<(), HapticsProviderError>;

    fn stop_rumble(&self, device_ids: &[String]) -> Result<(), HapticsProviderError>;
}
