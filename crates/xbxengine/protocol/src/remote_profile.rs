use serde::{Deserialize, Serialize};

use crate::runtime::XbxEngineTargetTypeDto;

/// 统一的 Xbox 远端画像基线：
/// - `session_target_type` 决定 Home / Cloud 一级语义
/// - `transport_path` 只在 Home 语义内细分 LAN / Relay
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum XbxEngineRemoteProfileKindDto {
    HomeLanGaming,
    CloudGaming,
    RelayGaming,
}

/// 基线画像上的运行态子画像，用于运行期观测与策略诊断透出。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum XbxEngineRemoteSubprofileKindDto {
    Steady,
    CloudStartup,
    CloudHighRtt,
    DecoderConstrained,
    DisplayConstrained,
}

impl XbxEngineRemoteProfileKindDto {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HomeLanGaming => "homeLanGaming",
            Self::CloudGaming => "cloudGaming",
            Self::RelayGaming => "relayGaming",
        }
    }

    pub fn reason_prefix(self) -> &'static str {
        match self {
            Self::HomeLanGaming => "direct",
            Self::CloudGaming => "cloud",
            Self::RelayGaming => "relay",
        }
    }

    pub fn is_cloud(self) -> bool {
        matches!(self, Self::CloudGaming)
    }

    pub fn is_relay(self) -> bool {
        matches!(self, Self::RelayGaming)
    }

    pub fn resolve(
        session_target_type: Option<&XbxEngineTargetTypeDto>,
        transport_path: Option<&str>,
    ) -> Self {
        match session_target_type {
            Some(XbxEngineTargetTypeDto::Cloud) => Self::CloudGaming,
            Some(XbxEngineTargetTypeDto::Home) => {
                if is_relay_transport_path(transport_path) {
                    Self::RelayGaming
                } else {
                    Self::HomeLanGaming
                }
            }
            None => {
                if is_relay_transport_path(transport_path) {
                    Self::RelayGaming
                } else {
                    Self::HomeLanGaming
                }
            }
        }
    }

    pub fn from_target_type(session_target_type: XbxEngineTargetTypeDto) -> Self {
        Self::resolve(Some(&session_target_type), None)
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "homeLanGaming" => Some(Self::HomeLanGaming),
            "cloudGaming" => Some(Self::CloudGaming),
            "relayGaming" => Some(Self::RelayGaming),
            _ => None,
        }
    }
}

impl XbxEngineRemoteSubprofileKindDto {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Steady => "steady",
            Self::CloudStartup => "cloudStartup",
            Self::CloudHighRtt => "cloudHighRtt",
            Self::DecoderConstrained => "decoderConstrained",
            Self::DisplayConstrained => "displayConstrained",
        }
    }
}

pub fn compose_effective_remote_profile_label(
    baseline: XbxEngineRemoteProfileKindDto,
    dynamic: XbxEngineRemoteSubprofileKindDto,
) -> String {
    format!("{}+{}", baseline.as_str(), dynamic.as_str())
}

pub fn is_relay_transport_path(transport_path: Option<&str>) -> bool {
    transport_path
        .map(|path| path.to_ascii_lowercase().starts_with("relay"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{
        compose_effective_remote_profile_label, XbxEngineRemoteProfileKindDto,
        XbxEngineRemoteSubprofileKindDto,
    };
    use crate::runtime::XbxEngineTargetTypeDto;

    #[test]
    fn cloud_target_overrides_transport_path() {
        assert_eq!(
            XbxEngineRemoteProfileKindDto::resolve(
                Some(&XbxEngineTargetTypeDto::Cloud),
                Some("Relay")
            ),
            XbxEngineRemoteProfileKindDto::CloudGaming
        );
    }

    #[test]
    fn home_target_uses_relay_path_as_refinement() {
        assert_eq!(
            XbxEngineRemoteProfileKindDto::resolve(
                Some(&XbxEngineTargetTypeDto::Home),
                Some("Relay (turn)")
            ),
            XbxEngineRemoteProfileKindDto::RelayGaming
        );
        assert_eq!(
            XbxEngineRemoteProfileKindDto::resolve(
                Some(&XbxEngineTargetTypeDto::Home),
                Some("Direct (host->host)")
            ),
            XbxEngineRemoteProfileKindDto::HomeLanGaming
        );
    }

    #[test]
    fn home_target_defaults_to_home_lan_without_transport_path() {
        assert_eq!(
            XbxEngineRemoteProfileKindDto::from_target_type(XbxEngineTargetTypeDto::Home),
            XbxEngineRemoteProfileKindDto::HomeLanGaming
        );
    }

    #[test]
    fn dynamic_subprofile_string_contract_is_stable() {
        assert_eq!(XbxEngineRemoteSubprofileKindDto::Steady.as_str(), "steady");
        assert_eq!(
            XbxEngineRemoteSubprofileKindDto::CloudStartup.as_str(),
            "cloudStartup"
        );
        assert_eq!(
            XbxEngineRemoteSubprofileKindDto::CloudHighRtt.as_str(),
            "cloudHighRtt"
        );
        assert_eq!(
            XbxEngineRemoteSubprofileKindDto::DecoderConstrained.as_str(),
            "decoderConstrained"
        );
        assert_eq!(
            XbxEngineRemoteSubprofileKindDto::DisplayConstrained.as_str(),
            "displayConstrained"
        );
    }

    #[test]
    fn effective_profile_label_composes_baseline_and_dynamic() {
        assert_eq!(
            compose_effective_remote_profile_label(
                XbxEngineRemoteProfileKindDto::CloudGaming,
                XbxEngineRemoteSubprofileKindDto::CloudHighRtt
            ),
            "cloudGaming+cloudHighRtt"
        );
    }
}
