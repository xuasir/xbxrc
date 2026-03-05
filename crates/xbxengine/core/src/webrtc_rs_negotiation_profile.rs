use std::{env, sync::OnceLock};

const NEGOTIATION_BRANCH_ENV: &str = "XBXENGINE_NEGOTIATION_BRANCH";
const NEGOTIATION_RESOLUTION_ENV: &str = "XBXENGINE_NEGOTIATION_RESOLUTION";
const NEGOTIATION_BITRATE_ENV: &str = "XBXENGINE_NEGOTIATION_BITRATE_KBPS";
const NEGOTIATION_MIN_BITRATE_ENV: &str = "XBXENGINE_NEGOTIATION_MIN_BITRATE_KBPS";
const NEGOTIATION_MAX_BITRATE_ENV: &str = "XBXENGINE_NEGOTIATION_MAX_BITRATE_KBPS";
const NEGOTIATION_MAX_FR_ENV: &str = "XBXENGINE_NEGOTIATION_MAX_FRAME_RATE";

const RTCP_TEST_BRANCH: &str = "rtcp-test";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WebRtcRsNegotiationProfile {
    pub width: u32,
    pub height: u32,
    pub min_bitrate_kbps: u32,
    pub target_bitrate_kbps: u32,
    pub max_bitrate_kbps: u32,
    pub max_frame_rate: u32,
    pub rtcp_test_branch_enabled: bool,
}

impl Default for WebRtcRsNegotiationProfile {
    fn default() -> Self {
        Self {
            // 默认按 1081 语义走 1440p 档。
            width: 2560,
            height: 1440,
            min_bitrate_kbps: 20_000,
            target_bitrate_kbps: 30_000,
            max_bitrate_kbps: 60_000,
            max_frame_rate: 60,
            rtcp_test_branch_enabled: false,
        }
    }
}

pub(crate) fn current_webrtc_rs_negotiation_profile() -> &'static WebRtcRsNegotiationProfile {
    static PROFILE: OnceLock<WebRtcRsNegotiationProfile> = OnceLock::new();
    PROFILE.get_or_init(load_profile_from_env)
}

fn load_profile_from_env() -> WebRtcRsNegotiationProfile {
    let mut profile = WebRtcRsNegotiationProfile::default();
    let branch = env::var(NEGOTIATION_BRANCH_ENV)
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if branch != RTCP_TEST_BRANCH {
        return profile;
    }

    profile.rtcp_test_branch_enabled = true;
    if let Some((width, height)) = parse_resolution_override() {
        profile.width = width;
        profile.height = height;
    }

    if let Some((min_bitrate, target_bitrate, max_bitrate)) = parse_bitrate_override() {
        profile.min_bitrate_kbps = min_bitrate;
        profile.target_bitrate_kbps = target_bitrate;
        profile.max_bitrate_kbps = max_bitrate;
    }

    if let Some(max_frame_rate) = parse_u32_env(NEGOTIATION_MAX_FR_ENV) {
        profile.max_frame_rate = max_frame_rate.max(1);
    }

    eprintln!(
        "[xbxengine][webrtc-rs] negotiation test branch enabled resolution={}x{} bitrate(min/target/max)={}/{}/{} max-fr={}",
        profile.width,
        profile.height,
        profile.min_bitrate_kbps,
        profile.target_bitrate_kbps,
        profile.max_bitrate_kbps,
        profile.max_frame_rate
    );
    profile
}

fn parse_resolution_override() -> Option<(u32, u32)> {
    let raw = env::var(NEGOTIATION_RESOLUTION_ENV).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let split_index = trimmed.find('x').or_else(|| trimmed.find('X'));
    let Some(split_index) = split_index else {
        eprintln!(
            "[xbxengine][webrtc-rs] invalid {} value, expected WIDTHxHEIGHT: {}",
            NEGOTIATION_RESOLUTION_ENV, trimmed
        );
        return None;
    };

    let (width_raw, height_raw_with_sep) = trimmed.split_at(split_index);
    let height_raw = &height_raw_with_sep[1..];
    let width = width_raw.trim().parse::<u32>().ok();
    let height = height_raw.trim().parse::<u32>().ok();
    match (width, height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => Some((width, height)),
        _ => {
            eprintln!(
                "[xbxengine][webrtc-rs] invalid {} value, expected positive WIDTHxHEIGHT: {}",
                NEGOTIATION_RESOLUTION_ENV, trimmed
            );
            None
        }
    }
}

fn parse_bitrate_override() -> Option<(u32, u32, u32)> {
    let raw = env::var(NEGOTIATION_BITRATE_ENV).ok();
    let value = raw.as_deref().map(str::trim).unwrap_or_default();

    // 支持两种格式：
    // 1) `30000` => min/target/max 全部使用同一值
    // 2) `20000,30000,60000` => 分别指定 min/target/max
    let base = if value.is_empty() {
        None
    } else {
        let parts = value
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect::<Vec<&str>>();
        if parts.len() == 1 {
            parts[0]
                .parse::<u32>()
                .ok()
                .filter(|bitrate| *bitrate > 0)
                .map(|bitrate| (bitrate, bitrate, bitrate))
        } else if parts.len() == 3 {
            match (
                parts[0].parse::<u32>().ok(),
                parts[1].parse::<u32>().ok(),
                parts[2].parse::<u32>().ok(),
            ) {
                (Some(min), Some(target), Some(max))
                    if min > 0 && target > 0 && max > 0 && min <= target && target <= max =>
                {
                    Some((min, target, max))
                }
                _ => None,
            }
        } else {
            None
        }
    };

    let Some((base_min, base_target, base_max)) = base else {
        if !value.is_empty() {
            eprintln!(
                "[xbxengine][webrtc-rs] invalid {} value, expected `N` or `MIN,TARGET,MAX`: {}",
                NEGOTIATION_BITRATE_ENV, value
            );
        }
        return None;
    };

    let min = parse_u32_env(NEGOTIATION_MIN_BITRATE_ENV).unwrap_or(base_min);
    let max = parse_u32_env(NEGOTIATION_MAX_BITRATE_ENV).unwrap_or(base_max);
    if min == 0 || max == 0 || min > max {
        eprintln!(
            "[xbxengine][webrtc-rs] invalid min/max bitrate override min={} max={}",
            min, max
        );
        return None;
    }

    let target = base_target.clamp(min, max);
    Some((min, target, max))
}

fn parse_u32_env(env_name: &str) -> Option<u32> {
    let raw = env::var(env_name).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    match trimmed.parse::<u32>() {
        Ok(value) if value > 0 => Some(value),
        _ => {
            eprintln!(
                "[xbxengine][webrtc-rs] invalid {} value, expected positive integer: {}",
                env_name, trimmed
            );
            None
        }
    }
}
