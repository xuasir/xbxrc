use serde::Serialize;
use serde_json::{Map, Value};

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeFeatureCapabilities {
    pub native_media: bool,
    pub native_gamepad: bool,
}

pub fn runtime_feature_capabilities() -> RuntimeFeatureCapabilities {
    let desktop = cfg!(any(target_os = "windows", target_os = "macos"));
    RuntimeFeatureCapabilities {
        native_media: desktop,
        native_gamepad: desktop,
    }
}

pub fn effective_runtime_mode(
    config: &Map<String, Value>,
    capabilities: RuntimeFeatureCapabilities,
) -> &'static str {
    if capabilities.native_media
        && config.get("rust_media_preview").and_then(Value::as_bool) == Some(true)
    {
        "rust-owned"
    } else {
        "webrtc-direct"
    }
}

pub fn effective_gamepad_backend(
    config: &Map<String, Value>,
    capabilities: RuntimeFeatureCapabilities,
) -> &'static str {
    if capabilities.native_gamepad
        && config
            .get("native_gamepad_preview")
            .and_then(Value::as_bool)
            == Some(true)
    {
        "native"
    } else {
        "browser"
    }
}

pub fn require_native_feature(
    provider: &dyn super::ConfigProvider,
    media: bool,
) -> Result<(), String> {
    let config =
        provider.get_by_keys(&["rust_media_preview".into(), "native_gamepad_preview".into()])?;
    let values = config.as_object().ok_or("runtimeFeatureConfigInvalid")?;
    let capabilities = runtime_feature_capabilities();
    let enabled = if media {
        effective_runtime_mode(values, capabilities) == "rust-owned"
    } else {
        effective_gamepad_backend(values, capabilities) == "native"
    };
    enabled.then_some(()).ok_or_else(|| {
        if media {
            "nativeMediaPreviewDisabled"
        } else {
            "nativeGamepadPreviewDisabled"
        }
        .into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::config::config_policy::normalize_config;
    use serde_json::json;

    struct PreviewConfig(Value);
    impl crate::mods::config::ConfigProvider for PreviewConfig {
        fn get_force_region_ip(&self) -> String {
            unreachable!()
        }
        fn get_streaming_config(&self) -> crate::mods::streaming::types::StreamingConfigSnapshot {
            unreachable!()
        }
        fn get_by_keys(&self, _: &[String]) -> Result<Value, String> {
            Ok(self.0.clone())
        }
        fn set_by_patch(&self, _: &Map<String, Value>) -> Result<Value, String> {
            unreachable!()
        }
        fn get_groups(&self) -> Result<Value, String> {
            unreachable!()
        }
    }

    #[test]
    fn native_rpc_start_requires_its_own_preview_switch() {
        let closed =
            PreviewConfig(json!({"rust_media_preview":false,"native_gamepad_preview":false}));
        assert_eq!(
            require_native_feature(&closed, true).unwrap_err(),
            "nativeMediaPreviewDisabled"
        );
        assert_eq!(
            require_native_feature(&closed, false).unwrap_err(),
            "nativeGamepadPreviewDisabled"
        );
        let media =
            PreviewConfig(json!({"rust_media_preview":true,"native_gamepad_preview":false}));
        let gamepad =
            PreviewConfig(json!({"rust_media_preview":false,"native_gamepad_preview":true}));
        let caps = runtime_feature_capabilities();
        assert_eq!(
            require_native_feature(&media, true).is_ok(),
            caps.native_media
        );
        assert!(require_native_feature(&media, false).is_err());
        assert!(require_native_feature(&gamepad, true).is_err());
        assert_eq!(
            require_native_feature(&gamepad, false).is_ok(),
            caps.native_gamepad
        );
    }

    #[test]
    fn retired_selectors_are_removed_and_default_to_browser() {
        let config = normalize_config(
            json!({"stream_runtime_mode":"rust-owned", "gamepad_backend":"native", "polling_rate":1000,"use_vulkan":true})
                .as_object()
                .unwrap()
                .clone(),
        );
        assert!(!config.contains_key("stream_runtime_mode"));
        assert!(!config.contains_key("gamepad_backend"));
        assert!(!config.contains_key("polling_rate"));
        assert!(!config.contains_key("use_vulkan"));
        let caps = RuntimeFeatureCapabilities {
            native_media: true,
            native_gamepad: true,
        };
        assert_eq!(effective_runtime_mode(&config, caps), "webrtc-direct");
        assert_eq!(effective_gamepad_backend(&config, caps), "browser");
    }

    #[test]
    fn preview_features_require_independent_opt_in_and_capability() {
        for media in [false, true] {
            for gamepad in [false, true] {
                for native_media in [false, true] {
                    for native_gamepad in [false, true] {
                        let config =
                            json!({"rust_media_preview":media, "native_gamepad_preview":gamepad});
                        let caps = RuntimeFeatureCapabilities {
                            native_media,
                            native_gamepad,
                        };
                        assert_eq!(
                            effective_runtime_mode(config.as_object().unwrap(), caps),
                            if media && native_media {
                                "rust-owned"
                            } else {
                                "webrtc-direct"
                            }
                        );
                        assert_eq!(
                            effective_gamepad_backend(config.as_object().unwrap(), caps),
                            if gamepad && native_gamepad {
                                "native"
                            } else {
                                "browser"
                            }
                        );
                    }
                }
            }
        }
    }
}
