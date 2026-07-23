use crate::cloud_access::{replace_home_host_facts, HomeHostFacts};
use crate::{deserialize, resolve_web_token_claims, XboxBridgeError};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
use tokio::task::JoinSet;
use xbox_streaming::{HostAddr, RemoteConsoleSnapshot};
use xbox_webapi::{
    AchievementsApi, ConsoleApi, ConsolePowerResponse, SmartglassApi, TitleHubApi, UserStatsApi,
};

const ACHIEVEMENTS_PAGE_SIZE: u32 = 200;
const MAX_ACHIEVEMENT_PAGES: usize = 20;
const MAX_PLAYTIME_TITLES: usize = 100;
const MAX_CONCURRENT_PLAYTIME_REQUESTS: usize = 4;

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxAchievementProgress {
    pub unlocked_count: u32,
    pub total_count: u32,
    pub earned_gamerscore: u32,
    pub total_gamerscore: u32,
    pub percentage: u32,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxGame {
    pub title_id: String,
    pub name: String,
    pub artwork_url: Option<String>,
    pub hero_url: Option<String>,
    pub last_played_at: Option<String>,
    pub achievement_progress: Option<XboxAchievementProgress>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxPlaytime {
    pub title_id: String,
    pub minutes: Option<u64>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxAchievement {
    pub id: String,
    pub title_id: String,
    pub name: String,
    pub description: String,
    pub locked_description: String,
    pub image_url: Option<String>,
    pub is_secret: bool,
    pub is_unlocked: bool,
    pub gamerscore: u32,
    pub progress_percentage: Option<u32>,
    pub unlocked_at: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxHostStorageDevice {
    pub id: Option<String>,
    pub name: Option<String>,
    pub free_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxHost {
    pub id: Option<String>,
    pub device_id: Option<String>,
    pub server_id: Option<String>,
    pub name: Option<String>,
    pub device_name: Option<String>,
    pub locale: Option<String>,
    pub region: Option<String>,
    pub power_state: Option<String>,
    pub console_type: Option<String>,
    pub remote_management_enabled: Option<bool>,
    pub console_streaming_enabled: Option<bool>,
    pub wireless_warning: Option<bool>,
    pub out_of_home_warning: Option<bool>,
    pub storage_devices: Vec<XboxHostStorageDevice>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct XboxConsolePowerResult {
    pub console_id: String,
    pub accepted: bool,
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_hosts(web_token_json: String) -> Result<Vec<XboxHost>, XboxBridgeError> {
    let claims = claims_from_json(&web_token_json)?;
    let account_id = claims.xuid.clone().unwrap_or_else(|| claims.uhs.clone());
    let smartglass = SmartglassApi::new(claims.uhs, claims.token);
    let request = smartglass.get_consoles_list();
    match tokio::time::timeout(std::time::Duration::from_secs(8), request).await {
        Ok(Ok(response)) => {
            let hosts = extract_hosts(&response);
            replace_home_host_facts(&account_id, extract_home_host_facts(&response))?;
            Ok(hosts)
        }
        Ok(Err(_)) | Err(_) => Ok(Vec::new()),
    }
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn power_on_console(
    web_token_json: String,
    console_id: String,
) -> Result<XboxConsolePowerResult, XboxBridgeError> {
    send_console_power_command(web_token_json, console_id, ConsolePowerCommand::PowerOn).await
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn power_off_console(
    web_token_json: String,
    console_id: String,
) -> Result<XboxConsolePowerResult, XboxBridgeError> {
    send_console_power_command(web_token_json, console_id, ConsolePowerCommand::PowerOff).await
}

#[derive(Debug, Clone, Copy)]
enum ConsolePowerCommand {
    PowerOn,
    PowerOff,
}

async fn send_console_power_command(
    web_token_json: String,
    console_id: String,
    command: ConsolePowerCommand,
) -> Result<XboxConsolePowerResult, XboxBridgeError> {
    let console_id = normalize_console_id(&console_id)?;
    let claims = claims_from_json(&web_token_json)?;
    let api = ConsoleApi::new(claims.uhs, claims.token);
    let response = match command {
        ConsolePowerCommand::PowerOn => api.power_on(&console_id).await,
        ConsolePowerCommand::PowerOff => api.power_off(&console_id).await,
    }
    .map_err(|error| XboxBridgeError::Data(error.to_string()))?;

    Ok(map_console_power_response(response))
}

fn normalize_console_id(value: &str) -> Result<String, XboxBridgeError> {
    let console_id = value.trim();
    if console_id.is_empty() || console_id.len() > 256 || console_id.chars().any(char::is_control) {
        return Err(XboxBridgeError::InvalidData(
            "console id must be a bounded printable identifier".to_string(),
        ));
    }
    Ok(console_id.to_string())
}

fn map_console_power_response(response: ConsolePowerResponse) -> XboxConsolePowerResult {
    XboxConsolePowerResult {
        console_id: response.console_id,
        accepted: response.accepted,
    }
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_game_library(web_token_json: String) -> Result<Vec<XboxGame>, XboxBridgeError> {
    let claims = claims_from_json(&web_token_json)?;
    let xuid = require_xuid(&claims.xuid)?;
    let response = TitleHubApi::new(claims.uhs, claims.token)
        .get_title_history(xuid)
        .await
        .map_err(|error| XboxBridgeError::Data(error.to_string()))?;

    Ok(extract_games(&response))
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_playtimes(
    web_token_json: String,
    title_ids: Vec<String>,
) -> Result<Vec<XboxPlaytime>, XboxBridgeError> {
    let claims = claims_from_json(&web_token_json)?;
    let xuid = require_xuid(&claims.xuid)?;
    let title_ids = normalize_title_ids(title_ids)?;
    let api = UserStatsApi::new(claims.uhs, claims.token);
    let xuid = xuid.to_string();
    let mut resolved = HashMap::new();
    let mut tasks = JoinSet::new();
    let mut next_index = 0;

    while next_index < title_ids.len() && tasks.len() < MAX_CONCURRENT_PLAYTIME_REQUESTS {
        spawn_playtime_request(
            &mut tasks,
            api.clone(),
            xuid.clone(),
            title_ids[next_index].clone(),
        );
        next_index += 1;
    }

    while let Some(result) = tasks.join_next().await {
        match result {
            Ok((_, Err(error))) if error.to_status_code() == Some(400) => {}
            Ok((_, Err(error))) => return Err(XboxBridgeError::Data(error.to_string())),
            Ok((title_id, Ok(response))) => {
                resolved.extend(extract_playtimes(&response, Some(&title_id)))
            }
            Err(error) => {
                return Err(XboxBridgeError::Data(format!(
                    "playtime request task failed: {error}"
                )))
            }
        }

        if next_index < title_ids.len() {
            spawn_playtime_request(
                &mut tasks,
                api.clone(),
                xuid.clone(),
                title_ids[next_index].clone(),
            );
            next_index += 1;
        }
    }

    Ok(title_ids
        .into_iter()
        .map(|title_id| XboxPlaytime {
            minutes: resolved.get(&title_id).copied(),
            title_id,
        })
        .collect())
}

fn spawn_playtime_request(
    tasks: &mut JoinSet<(String, Result<Value, xbox_webapi::WebApiError>)>,
    api: UserStatsApi,
    xuid: String,
    title_id: String,
) {
    tasks.spawn(async move {
        let result = api.get_minutes_played(&xuid, &title_id).await;
        (title_id, result)
    });
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_achievements(
    web_token_json: String,
    title_id: String,
) -> Result<Vec<XboxAchievement>, XboxBridgeError> {
    validate_title_id(&title_id)?;
    let claims = claims_from_json(&web_token_json)?;
    let xuid = require_xuid(&claims.xuid)?;
    let api = AchievementsApi::new(claims.uhs, claims.token);
    let mut achievements = Vec::new();
    let mut continuation: Option<String> = None;
    let mut seen_tokens = HashSet::new();
    let mut seen_ids = HashSet::new();

    for _ in 0..MAX_ACHIEVEMENT_PAGES {
        let response = api
            .get_title_achievements(
                xuid,
                &title_id,
                continuation.as_deref(),
                Some(ACHIEVEMENTS_PAGE_SIZE),
            )
            .await
            .map_err(|error| XboxBridgeError::Data(error.to_string()))?;

        for achievement in extract_achievements(&response, &title_id) {
            if seen_ids.insert(achievement.id.clone()) {
                achievements.push(achievement);
            }
        }

        let Some(next) = continuation_token(&response) else {
            return Ok(achievements);
        };
        if !seen_tokens.insert(next.clone()) {
            return Err(XboxBridgeError::Data(
                "achievement pagination repeated a continuation token".to_string(),
            ));
        }
        continuation = Some(next);
    }

    Err(XboxBridgeError::Data(format!(
        "achievement pagination exceeded {MAX_ACHIEVEMENT_PAGES} pages"
    )))
}

fn claims_from_json(raw: &str) -> Result<crate::WebTokenClaims, XboxBridgeError> {
    let token: Value = deserialize(raw)?;
    resolve_web_token_claims(&token)
}

fn require_xuid(xuid: &Option<String>) -> Result<&str, XboxBridgeError> {
    xuid.as_deref()
        .ok_or_else(|| XboxBridgeError::InvalidData("web token is missing xuid".to_string()))
}

fn normalize_title_ids(title_ids: Vec<String>) -> Result<Vec<String>, XboxBridgeError> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for title_id in title_ids {
        let title_id = title_id.trim();
        validate_title_id(title_id)?;
        if seen.insert(title_id.to_string()) {
            normalized.push(title_id.to_string());
        }
    }
    if normalized.is_empty() {
        return Ok(normalized);
    }
    if normalized.len() > MAX_PLAYTIME_TITLES {
        return Err(XboxBridgeError::InvalidData(format!(
            "playtime request supports at most {MAX_PLAYTIME_TITLES} titles"
        )));
    }
    Ok(normalized)
}

fn validate_title_id(title_id: &str) -> Result<(), XboxBridgeError> {
    if title_id.is_empty() || !title_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(XboxBridgeError::InvalidData(
            "title id must be numeric".to_string(),
        ));
    }
    Ok(())
}

fn extract_games(response: &Value) -> Vec<XboxGame> {
    root(response)
        .get("titles")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(map_game)
        .collect()
}

fn extract_hosts(response: &Value) -> Vec<XboxHost> {
    find_host_values(response, 0)
        .unwrap_or_default()
        .into_iter()
        .filter_map(map_host)
        .collect()
}

fn find_host_values(value: &Value, depth: usize) -> Option<Vec<&Value>> {
    if depth > 5 {
        return None;
    }
    if let Some(items) = value.as_array() {
        let hosts = items
            .iter()
            .filter(|item| map_host(item).is_some())
            .collect::<Vec<_>>();
        return (!hosts.is_empty()).then_some(hosts);
    }
    let object = value.as_object()?;
    for key in [
        "results", "result", "devices", "consoles", "items", "data", "response", "body",
    ] {
        if let Some(hosts) = object
            .get(key)
            .and_then(|child| find_host_values(child, depth + 1))
        {
            return Some(hosts);
        }
    }
    None
}

pub(crate) fn extract_home_host_facts(response: &Value) -> Vec<HomeHostFacts> {
    find_host_values(response, 0)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|raw| {
            let host = map_host(raw)?;
            let console_addrs = extract_console_addrs(raw);
            Some(HomeHostFacts {
                remote_console: RemoteConsoleSnapshot {
                    id: host.id,
                    device_id: host.device_id,
                    server_id: host.server_id,
                    power_state: host.power_state,
                    remote_management_enabled: host.remote_management_enabled,
                    console_streaming_enabled: host.console_streaming_enabled,
                    console_addrs_count: console_addrs.len() as u32,
                    ready_source: Some("smartglass".to_string()),
                },
                console_addrs,
            })
        })
        .collect()
}

fn extract_console_addrs(raw: &Value) -> Vec<HostAddr> {
    fn visit(value: &Value, depth: usize, output: &mut Vec<HostAddr>) {
        if depth > 6 {
            return;
        }
        let Some(object) = value.as_object() else {
            return;
        };
        for (key, child) in object {
            if matches!(
                key.as_str(),
                "streamingEndpoints"
                    | "streamingAddresses"
                    | "streamingCandidates"
                    | "endpointCandidates"
                    | "consoleAddresses"
            ) {
                collect_addr_candidates(child, output);
            } else if matches!(
                key.as_str(),
                "remotePlay"
                    | "streaming"
                    | "endpoints"
                    | "network"
                    | "configuration"
                    | "connection"
            ) {
                match child {
                    Value::Object(_) => visit(child, depth + 1, output),
                    Value::Array(items) => {
                        for item in items {
                            visit(item, depth + 1, output);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn collect_addr_candidates(value: &Value, output: &mut Vec<HostAddr>) {
        match value {
            Value::Array(items) => {
                for item in items {
                    if let Some(addr) = parse_host_addr(item) {
                        output.push(addr);
                    }
                }
            }
            Value::Object(map) => {
                if let Some(addr) = parse_host_addr(value) {
                    output.push(addr);
                } else {
                    for nested in map.values() {
                        if let Some(addr) = parse_host_addr(nested) {
                            output.push(addr);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn parse_host_addr(value: &Value) -> Option<HostAddr> {
        let object = value.as_object()?;
        let ip = ["ip", "ipAddress", "address", "host"]
            .into_iter()
            .find_map(|key| optional_string(object.get(key)))?;
        if ip.is_empty() || ip.chars().any(char::is_whitespace) {
            return None;
        }
        let port = ["port", "portNumber", "streamingPort"]
            .into_iter()
            .find_map(|key| object.get(key).and_then(value_u64))
            .and_then(|value| u16::try_from(value).ok())?;
        (port > 0).then_some(HostAddr { ip, port })
    }

    let mut found = Vec::new();
    visit(raw, 0, &mut found);
    let mut seen = BTreeSet::new();
    found
        .into_iter()
        .filter(|addr| seen.insert((addr.ip.clone(), addr.port)))
        .collect()
}

fn map_host(raw: &Value) -> Option<XboxHost> {
    let object = raw.as_object()?;
    let id = optional_string(object.get("id"));
    let device_id = optional_string(object.get("deviceId"));
    let server_id = optional_string(object.get("serverId"));
    let name = optional_string(object.get("name"));
    let device_name = optional_string(object.get("deviceName"));
    if id.is_none()
        && device_id.is_none()
        && server_id.is_none()
        && name.is_none()
        && device_name.is_none()
    {
        return None;
    }

    let storage_devices = object
        .get("storageDevices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|device| {
            let device = device.as_object()?;
            Some(XboxHostStorageDevice {
                id: optional_string(device.get("storageDeviceId").or_else(|| device.get("id"))),
                name: optional_string(
                    device
                        .get("storageDeviceName")
                        .or_else(|| device.get("name")),
                ),
                free_bytes: device
                    .get("freeSpaceBytes")
                    .or_else(|| device.get("freeBytes"))
                    .and_then(value_u64),
                total_bytes: device
                    .get("totalSpaceBytes")
                    .or_else(|| device.get("totalBytes"))
                    .and_then(value_u64),
            })
        })
        .collect();

    Some(XboxHost {
        id,
        device_id,
        server_id,
        name,
        device_name,
        locale: optional_string(object.get("locale")),
        region: optional_string(object.get("region")),
        power_state: optional_string(object.get("powerState")),
        console_type: optional_string(object.get("consoleType")),
        remote_management_enabled: object
            .get("remoteManagementEnabled")
            .and_then(Value::as_bool),
        console_streaming_enabled: object
            .get("consoleStreamingEnabled")
            .and_then(Value::as_bool),
        wireless_warning: object.get("wirelessWarning").and_then(Value::as_bool),
        out_of_home_warning: object.get("outOfHomeWarning").and_then(Value::as_bool),
        storage_devices,
    })
}

fn map_game(raw: &Value) -> Option<XboxGame> {
    if raw
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.eq_ignore_ascii_case("Game"))
    {
        return None;
    }

    let title_id = value_string(raw.get("titleId")?)?;
    let name = raw.get("name")?.as_str()?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    let images = raw.get("images").and_then(Value::as_array);
    let achievement_progress = raw
        .get("achievement")
        .filter(|value| value.is_object())
        .map(|achievement| XboxAchievementProgress {
            unlocked_count: value_u32(achievement.get("currentAchievements")).unwrap_or(0),
            total_count: value_u32(achievement.get("totalAchievements")).unwrap_or(0),
            earned_gamerscore: value_u32(achievement.get("currentGamerscore")).unwrap_or(0),
            total_gamerscore: value_u32(achievement.get("totalGamerscore")).unwrap_or(0),
            percentage: value_u32(achievement.get("progressPercentage"))
                .unwrap_or(0)
                .min(100),
        });

    Some(XboxGame {
        title_id,
        name,
        artwork_url: find_image(
            images,
            &["BoxArt", "Poster", "Tile", "FeaturePromotionalSquareArt"],
        )
        .or_else(|| optional_string(raw.get("displayImage"))),
        hero_url: find_image(
            images,
            &[
                "Hero",
                "SuperHeroArt",
                "BrandedKeyArt",
                "WideBackgroundImage",
            ],
        ),
        last_played_at: raw
            .get("titleHistory")
            .and_then(|value| value.get("lastTimePlayed"))
            .and_then(|value| optional_string(Some(value))),
        achievement_progress,
    })
}

fn find_image(images: Option<&Vec<Value>>, preferred_types: &[&str]) -> Option<String> {
    let images = images?;
    preferred_types.iter().find_map(|preferred| {
        images.iter().find_map(|image| {
            let image_type = image.get("type")?.as_str()?;
            if image_type.eq_ignore_ascii_case(preferred) {
                optional_string(image.get("url"))
            } else {
                None
            }
        })
    })
}

fn extract_achievements(response: &Value, title_id: &str) -> Vec<XboxAchievement> {
    root(response)
        .get("achievements")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|raw| map_achievement(raw, title_id))
        .collect()
}

fn map_achievement(raw: &Value, title_id: &str) -> Option<XboxAchievement> {
    let id = value_string(raw.get("id")?)?;
    let name = optional_string(raw.get("name")).unwrap_or_else(|| "Xbox Achievement".to_string());
    let is_unlocked = raw
        .get("progressState")
        .and_then(Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case("Achieved"));
    let progression = raw.get("progression");

    Some(XboxAchievement {
        id,
        title_id: title_id.to_string(),
        name,
        description: optional_string(raw.get("description")).unwrap_or_default(),
        locked_description: optional_string(raw.get("lockedDescription")).unwrap_or_default(),
        image_url: raw
            .get("mediaAssets")
            .and_then(Value::as_array)
            .and_then(|assets| {
                assets
                    .iter()
                    .find(|asset| {
                        asset
                            .get("type")
                            .and_then(Value::as_str)
                            .is_some_and(|value| value.eq_ignore_ascii_case("Icon"))
                    })
                    .or_else(|| assets.first())
            })
            .and_then(|asset| optional_string(asset.get("url"))),
        is_secret: raw
            .get("isSecret")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        is_unlocked,
        gamerscore: achievement_gamerscore(raw),
        progress_percentage: achievement_progress_percentage(progression, is_unlocked),
        unlocked_at: if is_unlocked {
            progression
                .and_then(|value| value.get("timeUnlocked"))
                .and_then(|value| optional_string(Some(value)))
        } else {
            None
        },
    })
}

fn achievement_gamerscore(raw: &Value) -> u32 {
    raw.get("rewards")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|reward| {
            reward
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|value| value.eq_ignore_ascii_case("Gamerscore"))
        })
        .and_then(|reward| value_u32(reward.get("value")))
        .unwrap_or(0)
}

fn achievement_progress_percentage(progression: Option<&Value>, unlocked: bool) -> Option<u32> {
    if unlocked {
        return Some(100);
    }
    let requirement = progression?.get("requirements")?.as_array()?.first()?;
    let current = value_f64(requirement.get("current"))?;
    let target = value_f64(requirement.get("target"))?;
    if !current.is_finite() || !target.is_finite() || target <= 0.0 {
        return None;
    }
    Some(((current / target) * 100.0).clamp(0.0, 100.0).round() as u32)
}

fn continuation_token(response: &Value) -> Option<String> {
    root(response)
        .get("pagingInfo")
        .and_then(|value| value.get("continuationToken"))
        .and_then(|value| optional_string(Some(value)))
}

fn extract_playtimes(response: &Value, requested_title_id: Option<&str>) -> HashMap<String, u64> {
    let root = root(response);
    let mut values = HashMap::new();

    if let Some(stats) = root.get("statlistscollection").and_then(Value::as_array) {
        for collection in stats {
            record_playtime(collection, requested_title_id, &mut values);
            if let Some(nested_stats) = collection.get("stats").and_then(Value::as_array) {
                for stat in nested_stats {
                    record_playtime(stat, requested_title_id, &mut values);
                }
            }
        }
    }

    if let Some(groups) = root.get("groups").and_then(Value::as_array) {
        for group in groups {
            let group_title_id = group
                .get("titleid")
                .or_else(|| group.get("titleId"))
                .and_then(value_string);
            let Some(collections) = group.get("statlistscollection").and_then(Value::as_array)
            else {
                continue;
            };
            for collection in collections {
                let Some(stats) = collection.get("stats").and_then(Value::as_array) else {
                    continue;
                };
                for stat in stats {
                    record_playtime(stat, group_title_id.as_deref(), &mut values);
                }
            }
        }
    }

    values
}

fn record_playtime(
    stat: &Value,
    fallback_title_id: Option<&str>,
    output: &mut HashMap<String, u64>,
) {
    let is_minutes_played = stat
        .get("name")
        .and_then(Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case("MinutesPlayed"));
    if !is_minutes_played {
        return;
    }
    let title_id = stat
        .get("titleid")
        .or_else(|| stat.get("titleId"))
        .and_then(value_string)
        .or_else(|| fallback_title_id.map(str::to_string));
    let minutes = stat.get("value").and_then(value_u64);
    if let (Some(title_id), Some(minutes)) = (title_id, minutes) {
        output.insert(title_id, minutes);
    }
}

fn root(value: &Value) -> &Value {
    value.get("data").unwrap_or(value)
}

fn optional_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(value_string)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn value_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn value_u32(value: Option<&Value>) -> Option<u32> {
    value_u64(value?).and_then(|value| u32::try_from(value).ok())
}

fn value_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64().or_else(|| {
            number
                .as_f64()
                .filter(|value| value.is_finite() && *value >= 0.0)
                .map(|value| value.floor() as u64)
        }),
        Value::String(value) => value.parse::<u64>().ok().or_else(|| {
            value
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite() && *value >= 0.0)
                .map(|value| value.floor() as u64)
        }),
        _ => None,
    }
}

fn value_f64(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(value) => value.parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        extract_achievements, extract_games, extract_home_host_facts, extract_hosts,
        extract_playtimes, map_console_power_response, normalize_console_id, normalize_title_ids,
        power_off_console, power_on_console,
    };
    use serde_json::json;
    use xbox_webapi::ConsolePowerResponse;

    #[test]
    fn maps_game_history_and_filters_apps() {
        let games = extract_games(&json!({
            "titles": [
                {
                    "titleId": 1292135258u64,
                    "name": "Halo Infinite",
                    "type": "Game",
                    "displayImage": "https://example.invalid/fallback.png",
                    "images": [
                        {"type": "Hero", "url": "https://example.invalid/hero.png"},
                        {"type": "BoxArt", "url": "https://example.invalid/box.png"}
                    ],
                    "achievement": {
                        "currentAchievements": 10,
                        "totalAchievements": 50,
                        "currentGamerscore": 200,
                        "totalGamerscore": 1000,
                        "progressPercentage": 20
                    },
                    "titleHistory": {"lastTimePlayed": "2026-07-01T08:00:00Z"}
                },
                {"titleId": "1", "name": "System App", "type": "App"}
            ]
        }));

        assert_eq!(games.len(), 1);
        assert_eq!(games[0].title_id, "1292135258");
        assert_eq!(
            games[0].artwork_url.as_deref(),
            Some("https://example.invalid/box.png")
        );
        assert_eq!(
            games[0].achievement_progress.as_ref().unwrap().percentage,
            20
        );
    }

    #[test]
    fn maps_achievement_details() {
        let achievements = extract_achievements(
            &json!({
                "achievements": [{
                    "id": "1",
                    "name": "First Steps",
                    "description": "Complete the tutorial",
                    "lockedDescription": "Complete the tutorial",
                    "progressState": "Achieved",
                    "isSecret": false,
                    "progression": {"timeUnlocked": "2026-07-01T08:00:00Z"},
                    "mediaAssets": [{"type": "Icon", "url": "https://example.invalid/icon.png"}],
                    "rewards": [{"type": "Gamerscore", "value": "25"}]
                }]
            }),
            "1292135258",
        );

        assert_eq!(achievements.len(), 1);
        assert!(achievements[0].is_unlocked);
        assert_eq!(achievements[0].gamerscore, 25);
        assert_eq!(achievements[0].progress_percentage, Some(100));
    }

    #[test]
    fn maps_top_level_and_group_playtime_variants() {
        let values = extract_playtimes(
            &json!({
                "statlistscollection": [{
                    "titleid": "111",
                    "name": "MinutesPlayed",
                    "value": "1234"
                }, {
                    "stats": [{"name": "MinutesPlayed", "value": "90"}]
                }],
                "groups": [{
                    "titleId": "222",
                    "statlistscollection": [{
                        "stats": [{"name": "MinutesPlayed", "value": 45.9}]
                    }]
                }]
            }),
            Some("333"),
        );

        assert_eq!(values.get("111"), Some(&1234));
        assert_eq!(values.get("222"), Some(&45));
        assert_eq!(values.get("333"), Some(&90));
    }

    #[test]
    fn deduplicates_and_validates_playtime_title_ids() {
        let ids = normalize_title_ids(vec![
            " 123 ".to_string(),
            "123".to_string(),
            "456".to_string(),
        ])
        .expect("ids");

        assert_eq!(ids, vec!["123", "456"]);
        assert!(normalize_title_ids(vec!["game".to_string()]).is_err());
    }

    #[test]
    fn maps_nested_hosts_and_storage_variants() {
        let hosts = extract_hosts(&json!({
            "data": {"devices": [{
                "id": "console-command-id",
                "serverId": "stream-target-id",
                "deviceName": "客厅 Xbox",
                "consoleType": "Series X",
                "powerState": "ConnectedStandby",
                "remoteManagementEnabled": true,
                "consoleStreamingEnabled": true,
                "storageDevices": [{
                    "storageDeviceId": "internal",
                    "storageDeviceName": "内部存储",
                    "freeSpaceBytes": 250,
                    "totalSpaceBytes": 1000
                }]
            }]}
        }));

        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].server_id.as_deref(), Some("stream-target-id"));
        assert_eq!(hosts[0].storage_devices[0].free_bytes, Some(250));
        assert!(hosts[0].remote_management_enabled == Some(true));
    }

    #[test]
    fn maps_home_identity_capability_and_console_addresses() {
        let facts = extract_home_host_facts(&json!({
            "data": {"devices": [{
                "id": "command-id",
                "deviceId": "device-id",
                "serverId": "server-id",
                "powerState": "On",
                "remoteManagementEnabled": true,
                "consoleStreamingEnabled": true,
                "remotePlay": {
                    "endpointCandidates": [
                        {"ip": "10.0.0.8", "port": 9002},
                        {"ipAddress": "10.0.0.8", "portNumber": "9002"}
                    ]
                }
            }]}
        }));

        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].canonical_target_id(), Some("server-id"));
        assert!(facts[0].matches("command-id"));
        assert!(facts[0].matches("device-id"));
        assert_eq!(facts[0].console_addrs.len(), 1);
        assert_eq!(facts[0].console_addrs[0].ip, "10.0.0.8");
        assert_eq!(facts[0].remote_console.console_addrs_count, 1);
    }

    #[test]
    fn validates_and_normalizes_console_command_identity() {
        assert_eq!(
            normalize_console_id("  console-command-id  ").expect("console id"),
            "console-command-id"
        );
        assert!(normalize_console_id(" \n ").is_err());
        assert!(normalize_console_id(&"x".repeat(257)).is_err());
    }

    #[test]
    fn maps_console_power_result_for_uniffi() {
        let result = map_console_power_response(ConsolePowerResponse {
            console_id: "console-command-id".to_string(),
            accepted: true,
        });

        assert_eq!(result.console_id, "console-command-id");
        assert!(result.accepted);
    }

    #[tokio::test]
    async fn power_commands_reject_invalid_console_identity_before_network_access() {
        assert!(power_on_console("{}".to_string(), " \n ".to_string())
            .await
            .is_err());
        assert!(power_off_console("{}".to_string(), "x".repeat(257))
            .await
            .is_err());
    }
}
