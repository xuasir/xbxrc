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
const SCHEDULE_STARTUP_TIMEOUT_MS: u64 = 45_000;
const SCHEDULE_READY_TIMEOUT_MS: u64 = 45_000;
const SCHEDULE_RETRY_BACKOFF_MS: [u64; 3] = [1_000, 3_000, 5_000];

pub fn compile_session(
    config: &Config,
    context: &Context,
    access: &ResolvedSessionAccess,
    runtime_mode: RuntimeMode,
) -> SessionPlan {
    let locale = normalize_locale(&config.session.preferred_game_locale);
    let device = compile_device_profile(config, context.target);
    let ms_device_info = build_ms_device_info(&device);

    let mut headers = BTreeMap::new();
    headers.insert(
        "x-ms-device-info".to_string(),
        build_ms_device_info_header_value(&ms_device_info),
    );
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
            max_width: 1920,
            max_height: 1080,
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
        ResolutionPreference::P1080Hq => DeviceProfileKind::Tizen,
        ResolutionPreference::P1080 => DeviceProfileKind::Windows,
        ResolutionPreference::Auto | ResolutionPreference::P720 => DeviceProfileKind::Android,
    }
}

fn build_ms_device_info(device: &DeviceProfile) -> MsDeviceInfo {
    MsDeviceInfo {
        client_app_type: "browser".to_string(),
        device_make: "Microsoft".to_string(),
        device_model: "unknown".to_string(),
        sdk_type: "web".to_string(),
        os_name: device.os_name.clone(),
        os_version: None,
        browser_name: "unknown".to_string(),
        browser_version: None,
        display_width_pixels: device.max_width,
        display_height_pixels: device.max_height,
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

fn build_ms_device_info_header_value(info: &MsDeviceInfo) -> String {
    format!(
        "{{\"clientAppType\":\"{}\",\"deviceInfo\":{{\"make\":\"{}\",\"model\":\"{}\",\"sdktype\":\"{}\"}},\"os\":{{\"name\":\"{}\"}},\"browser\":{{\"browserName\":\"{}\"}},\"displayInfo\":{{\"dimensions\":{{\"widthInPixels\":{},\"heightInPixels\":{}}},\"pixelDensity\":{{\"dpiX\":{},\"dpiY\":{}}}}}}}",
        info.client_app_type,
        info.device_make,
        info.device_model,
        info.sdk_type,
        info.os_name,
        info.browser_name,
        info.display_width_pixels,
        info.display_height_pixels,
        info.dpi_x,
        info.dpi_y,
    )
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
}
