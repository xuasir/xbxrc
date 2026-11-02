use serde::{Deserialize, Serialize};

use crate::SessionTargetType;

/// 统一的 Xbox 远端画像基线：
/// - `session_target_type` 决定 Home / Cloud 一级语义
/// - `transport_path` 只在 Home 语义内细分 LAN / Relay
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RemoteProfileKind {
    HomeLanGaming,
    CloudGaming,
    RelayGaming,
}

/// 基线画像上的运行态子画像，用于运行期观测与策略诊断透出。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RemoteSubprofileKind {
    Steady,
    CloudStartup,
    CloudHighRtt,
    DecoderConstrained,
    DisplayConstrained,
}

impl RemoteProfileKind {
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
        session_target_type: Option<&SessionTargetType>,
        transport_path: Option<&str>,
    ) -> Self {
        match session_target_type {
            Some(SessionTargetType::Cloud) => Self::CloudGaming,
            Some(SessionTargetType::Home) => {
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

    pub fn from_target_type(session_target_type: SessionTargetType) -> Self {
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

impl RemoteSubprofileKind {
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
    baseline: RemoteProfileKind,
    dynamic: RemoteSubprofileKind,
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
    use super::{compose_effective_remote_profile_label, RemoteProfileKind, RemoteSubprofileKind};
    use crate::SessionTargetType;

    #[test]
    fn cloud_target_overrides_transport_path() {
        assert_eq!(
            RemoteProfileKind::resolve(Some(&SessionTargetType::Cloud), Some("Relay")),
            RemoteProfileKind::CloudGaming
        );
    }

    #[test]
    fn home_target_uses_relay_path_as_refinement() {
        assert_eq!(
            RemoteProfileKind::resolve(Some(&SessionTargetType::Home), Some("Relay (turn)")),
            RemoteProfileKind::RelayGaming
        );
        assert_eq!(
            RemoteProfileKind::resolve(Some(&SessionTargetType::Home), Some("Direct (host->host)")),
            RemoteProfileKind::HomeLanGaming
        );
    }

    #[test]
    fn home_target_defaults_to_home_lan_without_transport_path() {
        assert_eq!(
            RemoteProfileKind::from_target_type(SessionTargetType::Home),
            RemoteProfileKind::HomeLanGaming
        );
    }

    #[test]
    fn dynamic_subprofile_string_contract_is_stable() {
        assert_eq!(RemoteSubprofileKind::Steady.as_str(), "steady");
        assert_eq!(RemoteSubprofileKind::CloudStartup.as_str(), "cloudStartup");
        assert_eq!(RemoteSubprofileKind::CloudHighRtt.as_str(), "cloudHighRtt");
        assert_eq!(
            RemoteSubprofileKind::DecoderConstrained.as_str(),
            "decoderConstrained"
        );
        assert_eq!(
            RemoteSubprofileKind::DisplayConstrained.as_str(),
            "displayConstrained"
        );
    }

    #[test]
    fn effective_profile_label_composes_baseline_and_dynamic() {
        assert_eq!(
            compose_effective_remote_profile_label(
                RemoteProfileKind::CloudGaming,
                RemoteSubprofileKind::CloudHighRtt
            ),
            "cloudGaming+cloudHighRtt"
        );
    }
}
