use crate::{deserialize, resolve_web_token_claims, XboxBridgeError};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use tokio::task::JoinSet;
use xbox_webapi::{AchievementsApi, TitleHubApi, UserStatsApi};

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
    use super::{extract_achievements, extract_games, extract_playtimes, normalize_title_ids};
    use serde_json::json;

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
}
