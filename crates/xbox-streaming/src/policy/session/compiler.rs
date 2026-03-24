use std::collections::BTreeMap;

use crate::policy::config::Config;
use crate::policy::context::Context;
use crate::policy::runtime::RuntimeMode;
use crate::policy::session::{
    DeviceProfile, DeviceProfileKind, MsDeviceInfo, PlaySettings, ResolutionPreference,
    ResolvedSessionAccess, SessionPlan, SessionSchedulePlan,
};
use crate::policy::types::{CompileError, Region, Target};

const SCHEDULE_MONITOR_INTERVAL_MS: u64 = 1_000;
const SCHEDULE_KEEPALIVE_INTERVAL_MS: u64 = 30_000;
const SCHEDULE_OFFER_POLL_INTERVAL_MS: u64 = 1_000;
const SCHEDULE_ICE_POLL_INTERVAL_MS: u64 = 1_000;
// xHome 主机注册和慢启动实际可能明显超过 45 秒，这里保守放宽默认窗口。
const SCHEDULE_STARTUP_TIMEOUT_MS: u64 = 90_000;
const SCHEDULE_READY_TIMEOUT_MS: u64 = 90_000;
const SCHEDULE_RETRY_BACKOFF_MS: [u64; 3] = [1_000, 3_000, 5_000];
const SPOOFED_CHROMIUM_VERSION: &str = "140.0.3485.54";
const XBOX_WEB_CLIENT_APP_ID: &str = "www.xbox.com";
const SPOOFED_TIZEN_USER_AGENT: &str = "Mozilla/5.0 (SMART-TV; LINUX; Tizen 7.0) AppleWebKit/537.36 (KHTML, like Gecko) 140.0.3485.54/7.0 TV Safari/537.36 FC4A1DA2-711C-4E9C-BC7F-047AF8A672EA";
const SPOOFED_WINDOWS_EDGE_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.3485.54 Safari/537.36 Edg/140.0.3485.54";
const SPOOFED_ANDROID_USER_AGENT: &str = "Mozilla/5.0 (Linux; Android 14) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.3485.54 Mobile Safari/537.36";

struct DeviceHttpProfile {
    device_make: &'static str,
    device_model: &'static str,
    os_version: &'static str,
    browser_name: &'static str,
    browser_version: &'static str,
    user_agent: &'static str,
}

pub fn compile_session(
    config: &Config,
    context: &Context,
    access: &ResolvedSessionAccess,
    runtime_mode: RuntimeMode,
) -> SessionPlan {
    let locale = normalize_locale(&config.session.preferred_game_locale);
    let device = compile_device_profile(config, context.target);
    let ms_device_info = build_ms_device_info(context.target, &device);

    let mut headers = BTreeMap::new();
    headers.insert(
        "x-ms-device-info".to_string(),
        build_ms_device_info_header_value(context.target, &ms_device_info),
    );
    headers.insert("User-Agent".to_string(), build_user_agent(&device));
    if let Some(client_ip) = normalize_optional_string(&config.session.client_ip_override) {
        headers.insert("X-Forwarded-For".to_string(), client_ip);
    }

    let settings = PlaySettings {
        locale: locale.clone(),
        os_name: device.os_name.clone(),
        ..Default::default()
    };

    SessionPlan {
        target: context.target,
        target_id: context.target_id.clone(),
        base_url: access.base_url.clone(),
        region: access.region.clone(),
        locale,
        device,
        schedule: compile_session_schedule(config, context.target, runtime_mode),
        settings,
        ms_device_info,
        headers,
    }
}

pub fn resolve_session_access(
    config: &Config,
    context: &Context,
) -> Result<ResolvedSessionAccess, CompileError> {
    let gs_token = context
        .session
        .gs_token
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or(CompileError::MissingGsToken)?;
    let region = pick_region(&context.session.regions, config)?;
    let base_url = normalize_base_url(&region.base_uri)
        .ok_or_else(|| CompileError::InvalidBaseUrl(region.base_uri.clone()))?;

    Ok(ResolvedSessionAccess {
        gs_token,
        base_url,
        region: Some(region.clone()),
    })
}

pub fn pick_region<'a>(regions: &'a [Region], config: &Config) -> Result<&'a Region, CompileError> {
    if regions.is_empty() {
        return Err(CompileError::MissingRegions);
    }

    if let Some(force_region_ip) = normalize_optional_string(&config.session.force_region_ip) {
        if let Some(region) = regions
            .iter()
            .find(|item| item.base_uri.contains(&force_region_ip))
        {
            return Ok(region);
        }
    }

    if let Some(preferred_region_name) =
        normalize_optional_string(&config.session.preferred_region_name)
    {
        if let Some(region) = regions
            .iter()
            .find(|item| item.name == preferred_region_name)
        {
            return Ok(region);
        }
        if let Some(region) = regions
            .iter()
            .find(|item| item.name.eq_ignore_ascii_case(&preferred_region_name))
        {
            return Ok(region);
        }
    }

    regions
        .iter()
        .find(|item| item.is_default)
        .or_else(|| regions.first())
        .ok_or(CompileError::NoUsableRegion)
}

fn normalize_base_url(base_uri: &str) -> Option<String> {
    let trimmed = base_uri.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    let normalized = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };

    (normalized != "https://" && normalized != "http://").then_some(normalized)
}

fn normalize_locale(locale: &str) -> String {
    let trimmed = locale.trim();
    if trimmed.is_empty() {
        "en-US".to_string()
    } else {
        trimmed.to_string()
    }
}

fn compile_device_profile(config: &Config, target: Target) -> DeviceProfile {
    let kind = config
        .session
        .device_profile
        .clone()
        .unwrap_or_else(|| resolution_to_device_kind(select_resolution(config, target)));

    match kind {
        DeviceProfileKind::Android => DeviceProfile {
            kind: DeviceProfileKind::Android,
            os_name: "android".to_string(),
            max_width: 1280,
            max_height: 720,
        },
        DeviceProfileKind::Windows => DeviceProfile {
            kind: DeviceProfileKind::Windows,
            os_name: "windows".to_string(),
            max_width: 1920,
            max_height: 1080,
        },
        DeviceProfileKind::Tizen => DeviceProfile {
            kind: DeviceProfileKind::Tizen,
            os_name: "tizen".to_string(),
            max_width: resolve_tizen_resolution(config, target).0,
            max_height: resolve_tizen_resolution(config, target).1,
        },
        DeviceProfileKind::Custom { name } => DeviceProfile {
            kind: DeviceProfileKind::Custom { name: name.clone() },
            os_name: name,
            max_width: 1920,
            max_height: 1080,
        },
    }
}

fn select_resolution(config: &Config, target: Target) -> ResolutionPreference {
    match target {
        Target::Home => config.session.home_resolution,
        Target::Cloud => config.session.cloud_resolution,
    }
}

fn resolution_to_device_kind(resolution: ResolutionPreference) -> DeviceProfileKind {
    match resolution {
        ResolutionPreference::P1080Hq | ResolutionPreference::P1440 => DeviceProfileKind::Tizen,
        ResolutionPreference::P1080 => DeviceProfileKind::Windows,
        ResolutionPreference::Auto | ResolutionPreference::P720 => DeviceProfileKind::Android,
    }
}

fn resolve_tizen_resolution(config: &Config, target: Target) -> (u32, u32) {
    match select_resolution(config, target) {
        ResolutionPreference::P1440 => (2560, 1440),
        ResolutionPreference::P1080Hq
        | ResolutionPreference::Auto
        | ResolutionPreference::P720
        | ResolutionPreference::P1080 => (1920, 1080),
    }
}

fn build_ms_device_info(target: Target, device: &DeviceProfile) -> MsDeviceInfo {
    let profile = resolve_device_http_profile(device);
    let (display_width_pixels, display_height_pixels) = match target {
        // home 画像必须跟随当前 device profile，不能再写死成 1080p。
        Target::Home => (device.max_width, device.max_height),
        Target::Cloud => (4096, 2160),
    };

    MsDeviceInfo {
        client_app_type: "browser".to_string(),
        device_make: profile.device_make.to_string(),
        device_model: profile.device_model.to_string(),
        sdk_type: "web".to_string(),
        os_name: device.os_name.clone(),
        os_version: Some(profile.os_version.to_string()),
        browser_name: profile.browser_name.to_string(),
        browser_version: Some(profile.browser_version.to_string()),
        display_width_pixels,
        display_height_pixels,
        dpi_x: 1,
        dpi_y: 1,
    }
}

fn compile_session_schedule(
    config: &Config,
    target: Target,
    runtime_mode: RuntimeMode,
) -> SessionSchedulePlan {
    // 显式编译调度策略，避免关键语义仅依赖 SessionSchedulePlan::default()。
    let (monitor_interval_ms, keepalive_interval_ms) = compile_loop_intervals(target, runtime_mode);
    let (offer_poll_interval_ms, ice_poll_interval_ms) =
        compile_signaling_poll_intervals(target, runtime_mode);
    let (startup_timeout_ms, ready_timeout_ms) = compile_state_timeouts(target, runtime_mode);
    let retry_backoff_ms = compile_retry_backoff(target, runtime_mode);
    let wake_console = target.is_home() && config.session.power_on;

    SessionSchedulePlan {
        monitor_interval_ms,
        keepalive_interval_ms,
        offer_poll_interval_ms,
        ice_poll_interval_ms,
        startup_timeout_ms,
        ready_timeout_ms,
        retry_backoff_ms,
        wake_console,
        require_console_ready: wake_console,
    }
}

fn compile_loop_intervals(target: Target, runtime_mode: RuntimeMode) -> (u64, u64) {
    match (target, runtime_mode) {
        (Target::Home, RuntimeMode::WebRtcDirect)
        | (Target::Home, RuntimeMode::RustOwned)
        | (Target::Cloud, RuntimeMode::WebRtcDirect)
        | (Target::Cloud, RuntimeMode::RustOwned) => {
            (SCHEDULE_MONITOR_INTERVAL_MS, SCHEDULE_KEEPALIVE_INTERVAL_MS)
        }
    }
}

fn compile_signaling_poll_intervals(target: Target, runtime_mode: RuntimeMode) -> (u64, u64) {
    match (target, runtime_mode) {
        (Target::Home, RuntimeMode::WebRtcDirect)
        | (Target::Home, RuntimeMode::RustOwned)
        | (Target::Cloud, RuntimeMode::WebRtcDirect)
        | (Target::Cloud, RuntimeMode::RustOwned) => (
            SCHEDULE_OFFER_POLL_INTERVAL_MS,
            SCHEDULE_ICE_POLL_INTERVAL_MS,
        ),
    }
}

fn compile_state_timeouts(target: Target, runtime_mode: RuntimeMode) -> (u64, u64) {
    match (target, runtime_mode) {
        (Target::Home, RuntimeMode::WebRtcDirect)
        | (Target::Home, RuntimeMode::RustOwned)
        | (Target::Cloud, RuntimeMode::WebRtcDirect)
        | (Target::Cloud, RuntimeMode::RustOwned) => {
            (SCHEDULE_STARTUP_TIMEOUT_MS, SCHEDULE_READY_TIMEOUT_MS)
        }
    }
}

fn compile_retry_backoff(target: Target, runtime_mode: RuntimeMode) -> Vec<u64> {
    match (target, runtime_mode) {
        (Target::Home, RuntimeMode::WebRtcDirect)
        | (Target::Home, RuntimeMode::RustOwned)
        | (Target::Cloud, RuntimeMode::WebRtcDirect)
        | (Target::Cloud, RuntimeMode::RustOwned) => SCHEDULE_RETRY_BACKOFF_MS.to_vec(),
    }
}

fn build_ms_device_info_header_value(target: Target, info: &MsDeviceInfo) -> String {
    let _ = target;
    let client_app_id = XBOX_WEB_CLIENT_APP_ID;

    serde_json::json!({
        "appInfo": {
            "env": {
                "clientAppId": client_app_id,
                "clientAppType": info.client_app_type,
                "clientAppVersion": "29.11.13-hotfix.3",
                "clientSdkVersion": "10.6.33",
                "httpEnvironment": "prod",
                "sdkInstallId": ""
            }
        },
        "dev": {
            "os": {
                "name": info.os_name,
                "ver": info.os_version.clone().unwrap_or_default(),
                "platform": "desktop"
            },
            "hw": {
                "make": info.device_make,
                "model": info.device_model,
                "sdktype": info.sdk_type
            },
            "browser": {
                "browserName": info.browser_name,
                "browserVersion": info.browser_version.clone().unwrap_or_default()
            },
            "displayInfo": {
                "dimensions": {
                    "widthInPixels": info.display_width_pixels,
                    "heightInPixels": info.display_height_pixels
                },
                "pixelDensity": {
                    "dpiX": info.dpi_x,
                    "dpiY": info.dpi_y
                }
            }
        }
    })
    .to_string()
}

fn build_user_agent(device: &DeviceProfile) -> String {
    resolve_device_http_profile(device).user_agent.to_string()
}

fn resolve_device_http_profile(device: &DeviceProfile) -> DeviceHttpProfile {
    match device.kind {
        DeviceProfileKind::Android => DeviceHttpProfile {
            device_make: "Google",
            device_model: "Pixel 8",
            os_version: "14",
            browser_name: "chrome",
            browser_version: SPOOFED_CHROMIUM_VERSION,
            user_agent: SPOOFED_ANDROID_USER_AGENT,
        },
        DeviceProfileKind::Windows => DeviceHttpProfile {
            device_make: "Microsoft",
            device_model: "Windows Desktop",
            os_version: "10.0.22631.2715",
            browser_name: "edge",
            browser_version: SPOOFED_CHROMIUM_VERSION,
            user_agent: SPOOFED_WINDOWS_EDGE_USER_AGENT,
        },
        DeviceProfileKind::Tizen => DeviceHttpProfile {
            device_make: "Samsung",
            device_model: "Samsung Smart TV",
            os_version: "7.0",
            browser_name: "chrome",
            browser_version: SPOOFED_CHROMIUM_VERSION,
            user_agent: SPOOFED_TIZEN_USER_AGENT,
        },
        DeviceProfileKind::Custom { .. } => DeviceHttpProfile {
            device_make: "Microsoft",
            device_model: "Custom Desktop",
            os_version: "10.0.22631.2715",
            browser_name: "edge",
            browser_version: SPOOFED_CHROMIUM_VERSION,
            user_agent: SPOOFED_WINDOWS_EDGE_USER_AGENT,
        },
    }
}

#[cfg(test)]
fn parse_header_json(value: &str) -> serde_json::Value {
    serde_json::from_str(value).expect("valid x-ms-device-info json")
}

#[cfg(test)]
fn header_str<'a>(json: &'a serde_json::Value, path: &[&str]) -> &'a str {
    let mut current = json;
    for segment in path {
        current = &current[*segment];
    }
    current.as_str().expect("json string field")
}

#[cfg(test)]
fn header_u64(json: &serde_json::Value, path: &[&str]) -> u64 {
    let mut current = json;
    for segment in path {
        current = &current[*segment];
    }
    current.as_u64().expect("json number field")
}

fn normalize_optional_string(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::session::ResolvedSessionAccess;

    fn sample_access() -> ResolvedSessionAccess {
        ResolvedSessionAccess {
            gs_token: "token".to_string(),
            base_url: "https://example.com".to_string(),
            region: None,
        }
    }

    #[test]
    fn schedule_is_explicitly_compiled_for_home_with_power_on() {
        let mut config = Config::default();
        config.session.power_on = true;

        let mut context = Context::default();
        context.target = Target::Home;
        context.target_id = "console-1".to_string();

        let plan = compile_session(
            &config,
            &context,
            &sample_access(),
            RuntimeMode::WebRtcDirect,
        );
        assert_eq!(
            plan.schedule.monitor_interval_ms,
            SCHEDULE_MONITOR_INTERVAL_MS
        );
        assert_eq!(
            plan.schedule.keepalive_interval_ms,
            SCHEDULE_KEEPALIVE_INTERVAL_MS
        );
        assert_eq!(
            plan.schedule.offer_poll_interval_ms,
            SCHEDULE_OFFER_POLL_INTERVAL_MS
        );
        assert_eq!(
            plan.schedule.ice_poll_interval_ms,
            SCHEDULE_ICE_POLL_INTERVAL_MS
        );
        assert_eq!(
            plan.schedule.startup_timeout_ms,
            SCHEDULE_STARTUP_TIMEOUT_MS
        );
        assert_eq!(plan.schedule.ready_timeout_ms, SCHEDULE_READY_TIMEOUT_MS);
        assert_eq!(
            plan.schedule.retry_backoff_ms,
            SCHEDULE_RETRY_BACKOFF_MS.to_vec()
        );
        assert!(plan.schedule.wake_console);
        assert!(plan.schedule.require_console_ready);
    }

    #[test]
    fn schedule_does_not_wake_console_for_cloud() {
        let mut config = Config::default();
        config.session.power_on = true;

        let mut context = Context::default();
        context.target = Target::Cloud;
        context.target_id = "title-1".to_string();

        let plan = compile_session(&config, &context, &sample_access(), RuntimeMode::RustOwned);
        assert!(!plan.schedule.wake_console);
        assert!(!plan.schedule.require_console_ready);
    }

    #[test]
    fn home_session_headers_include_user_agent_and_follow_home_resolution() {
        let mut config = Config::default();
        config.session.home_resolution = ResolutionPreference::P1080;

        let mut context = Context::default();
        context.target = Target::Home;
        context.target_id = "console-1".to_string();

        let plan = compile_session(
            &config,
            &context,
            &sample_access(),
            RuntimeMode::WebRtcDirect,
        );
        let header = parse_header_json(
            plan.headers
                .get("x-ms-device-info")
                .expect("x-ms-device-info header"),
        );

        assert_eq!(
            header_str(&header, &["appInfo", "env", "clientAppId"]),
            XBOX_WEB_CLIENT_APP_ID
        );
        assert_eq!(
            header_str(&header, &["dev", "browser", "browserVersion"]),
            SPOOFED_CHROMIUM_VERSION
        );
        assert_eq!(
            header_str(&header, &["dev", "browser", "browserName"]),
            "edge"
        );
        assert_eq!(
            header_str(&header, &["dev", "hw", "model"]),
            "Windows Desktop"
        );
        assert_eq!(
            header_u64(
                &header,
                &["dev", "displayInfo", "dimensions", "widthInPixels"]
            ),
            1920
        );
        assert_eq!(
            header_u64(
                &header,
                &["dev", "displayInfo", "dimensions", "heightInPixels"]
            ),
            1080
        );
        assert_eq!(
            plan.ms_device_info.browser_version.as_deref(),
            Some(SPOOFED_CHROMIUM_VERSION)
        );
        assert_eq!(
            plan.headers.get("User-Agent").map(String::as_str),
            Some(SPOOFED_WINDOWS_EDGE_USER_AGENT)
        );
    }

    #[test]
    fn home_session_display_target_is_dynamic_for_1440_profile() {
        let mut config = Config::default();
        config.session.home_resolution = ResolutionPreference::P1440;

        let mut context = Context::default();
        context.target = Target::Home;
        context.target_id = "console-1".to_string();

        let plan = compile_session(
            &config,
            &context,
            &sample_access(),
            RuntimeMode::WebRtcDirect,
        );
        let header = parse_header_json(
            plan.headers
                .get("x-ms-device-info")
                .expect("x-ms-device-info header"),
        );

        assert_eq!(plan.device.max_width, 2560);
        assert_eq!(plan.device.max_height, 1440);
        assert_eq!(
            header_u64(
                &header,
                &["dev", "displayInfo", "dimensions", "widthInPixels"]
            ),
            2560
        );
        assert_eq!(
            header_u64(
                &header,
                &["dev", "displayInfo", "dimensions", "heightInPixels"]
            ),
            1440
        );
        assert_eq!(
            plan.headers.get("User-Agent").map(String::as_str),
            Some(SPOOFED_TIZEN_USER_AGENT)
        );
    }

    #[test]
    fn cloud_session_headers_keep_custom_image() {
        let config = Config::default();

        let mut context = Context::default();
        context.target = Target::Cloud;
        context.target_id = "title-1".to_string();

        let plan = compile_session(&config, &context, &sample_access(), RuntimeMode::RustOwned);
        let header = parse_header_json(
            plan.headers
                .get("x-ms-device-info")
                .expect("x-ms-device-info header"),
        );

        assert_eq!(
            header_str(&header, &["appInfo", "env", "clientAppId"]),
            XBOX_WEB_CLIENT_APP_ID
        );
        assert_eq!(
            header_str(&header, &["dev", "browser", "browserVersion"]),
            SPOOFED_CHROMIUM_VERSION
        );
        assert_eq!(
            header_u64(
                &header,
                &["dev", "displayInfo", "dimensions", "widthInPixels"]
            ),
            4096
        );
        assert_eq!(
            header_u64(
                &header,
                &["dev", "displayInfo", "dimensions", "heightInPixels"]
            ),
            2160
        );
        assert_eq!(
            plan.headers.get("User-Agent").map(String::as_str),
            Some(SPOOFED_ANDROID_USER_AGENT)
        );
        assert_eq!(header_str(&header, &["dev", "hw", "model"]), "Pixel 8");
    }
}
