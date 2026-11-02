use serde::Serialize;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeFeatureCapabilities {
    pub native_media: bool,
    pub native_gamepad: bool,
}

/// 稳定发行只装配 Browser 输入与媒体；能力来自本包实现。
pub fn runtime_feature_capabilities() -> RuntimeFeatureCapabilities {
    RuntimeFeatureCapabilities {
        native_media: false,
        native_gamepad: false,
    }
}
