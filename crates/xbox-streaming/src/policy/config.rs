use serde::{Deserialize, Serialize};

use crate::policy::input::InputConfig;
use crate::policy::negotiation::NegotiationConfig;
use crate::policy::render::RenderConfig;
use crate::policy::runtime::RuntimeConfig;
use crate::policy::session::{ResolutionPreference, SessionConfig};

/// 串流配置只表达“偏好”，允许保留 Auto/Default 这类未决值。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub session: SessionConfig,
    pub negotiation: NegotiationConfig,
    pub input: InputConfig,
    pub runtime: RuntimeConfig,
    pub render: RenderConfig,
}

pub fn normalize_optional(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

impl Config {
    pub fn new_home_config(
        preferred_game_locale: String,
        force_region_ip: String,
        home_resolution: i64,
    ) -> Self {
        let mut config = Self::default();
        config.update_from_raw_values(
            Some(preferred_game_locale),
            normalize_optional(&force_region_ip),
            false,
            home_resolution,
        );
        config
    }

    pub fn update_from_raw_values(
        &mut self,
        preferred_game_locale: Option<String>,
        force_region_ip: Option<String>,
        ipv6: bool,
        resolution: i64,
    ) {
        if let Some(locale) = preferred_game_locale {
            self.session.preferred_game_locale = locale;
        }
        self.session.force_region_ip = force_region_ip;
        self.negotiation.cloud_prefer_ipv6 = ipv6;
        self.negotiation.home_prefer_ipv6 = ipv6;
        let pref = parse_resolution_preference(resolution);
        self.session.cloud_resolution = pref;
        self.session.home_resolution = pref;
    }
}

/// 将持久化配置里的数值分辨率映射为策略层偏好，避免多处手写同一分支。
pub fn parse_resolution_preference(resolution: i64) -> ResolutionPreference {
    match resolution {
        1440 => ResolutionPreference::P1440,
        1081 => ResolutionPreference::P1080Hq,
        1080 => ResolutionPreference::P1080,
        720 => ResolutionPreference::P720,
        _ => ResolutionPreference::Auto,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_resolution_preference;
    use crate::policy::session::ResolutionPreference;

    #[test]
    fn parses_resolution_preferences_from_raw_values() {
        assert_eq!(
            parse_resolution_preference(1440),
            ResolutionPreference::P1440
        );
        assert_eq!(
            parse_resolution_preference(1081),
            ResolutionPreference::P1080Hq
        );
        assert_eq!(
            parse_resolution_preference(1080),
            ResolutionPreference::P1080
        );
        assert_eq!(parse_resolution_preference(720), ResolutionPreference::P720);
        assert_eq!(parse_resolution_preference(0), ResolutionPreference::Auto);
    }
}
