use crate::data::{
    XboxAchievement, XboxAchievementProgress, XboxAchievementRequirement, XboxPlaytime,
};
use crate::{deserialize, resolve_web_token_claims, XboxBridgeError, XboxFriend};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use tokio::task::JoinSet;
use xbox_webapi::{
    library_root, library_string, parse_achievement_page, parse_game_history, parse_minutes_played,
    validate_minutes_played_response, AchievementsApi, LibraryGame, ProfileApi, TitleHubApi,
    UserStatsApi, WebApiError, USERSTATS_MAX_TITLES_PER_REQUEST,
};

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxLibraryGame {
    pub title_id: String,
    pub product_id: Option<String>,
    pub scid: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub artwork_url: Option<String>,
    pub last_played_at: Option<String>,
    pub achievement_progress: Option<XboxAchievementProgress>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxGameHistoryPage {
    pub items: Vec<XboxLibraryGame>,
    pub continuation: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxAchievementPage {
    pub items: Vec<XboxAchievement>,
    pub continuation: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxPlayerPlaytimes {
    pub items: Vec<XboxPlaytime>,
    pub failed_title_ids: Vec<String>,
    pub failures: Vec<crate::XboxSourceFailure>,
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_player_profile(
    web_token_json: String,
    target_xuid: String,
) -> Result<XboxFriend, XboxBridgeError> {
    let target = numeric_identity(&target_xuid)?;
    let claims = resolve_web_token_claims(&deserialize::<Value>(&web_token_json)?)?;
    let response = ProfileApi::new(claims.uhs, claims.token)
        .get_user(&target)
        .await
        .map_err(data_error)?;
    parse_player_profile(&response, &target)
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_game_history_page(
    web_token_json: String,
    target_xuid: Option<String>,
    continuation: Option<String>,
) -> Result<XboxGameHistoryPage, XboxBridgeError> {
    validate_continuation(continuation.as_deref())?;
    let claims = resolve_web_token_claims(&deserialize::<Value>(&web_token_json)?)?;
    let target = player_identity(target_xuid.as_deref(), claims.xuid.as_deref())?;
    let response = TitleHubApi::new(claims.uhs, claims.token)
        .get_title_history_page(&target, continuation.as_deref())
        .await
        .map_err(data_error)?;
    let page = parse_game_history(&response).map_err(data_error)?;
    Ok(XboxGameHistoryPage {
        items: page.items.into_iter().map(map_game).collect(),
        continuation: page.continuation,
    })
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_library_game(
    web_token_json: String,
    title_id: String,
    target_xuid: Option<String>,
) -> Result<Option<XboxLibraryGame>, XboxBridgeError> {
    let title = numeric_identity(&title_id)?;
    let claims = resolve_web_token_claims(&deserialize::<Value>(&web_token_json)?)?;
    let target = player_identity(target_xuid.as_deref(), claims.xuid.as_deref())?;
    let response = TitleHubApi::new(claims.uhs, claims.token)
        .get_title_detail(&target, &title)
        .await
        .map_err(data_error)?;
    let page = parse_game_history(&response).map_err(data_error)?;
    Ok(page
        .items
        .into_iter()
        .find(|game| game.xbox_title_id == title)
        .map(map_game))
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_achievement_page(
    web_token_json: String,
    title_id: String,
    target_xuid: Option<String>,
    locale: String,
    continuation: Option<String>,
) -> Result<XboxAchievementPage, XboxBridgeError> {
    validate_continuation(continuation.as_deref())?;
    let title = numeric_identity(&title_id)?;
    let claims = resolve_web_token_claims(&deserialize::<Value>(&web_token_json)?)?;
    let target = player_identity(target_xuid.as_deref(), claims.xuid.as_deref())?;
    let response = AchievementsApi::new(claims.uhs, claims.token, locale)
        .get_title_achievements(&target, &title, continuation.as_deref(), Some(200))
        .await
        .map_err(data_error)?;
    let page = parse_achievement_page(&response).map_err(data_error)?;
    Ok(XboxAchievementPage {
        items: page
            .items
            .into_iter()
            .map(|item| XboxAchievement {
                id: item.id,
                scid: item.scid,
                title_id: title.clone(),
                name: item.name,
                description: item.description,
                locked_description: item.locked_description,
                image_url: item.image_url,
                is_secret: item.is_secret,
                is_unlocked: item.is_unlocked,
                gamerscore: item.gamerscore.unwrap_or_default(),
                progress_percentage: item
                    .progress_percentage
                    .map(|value| value.clamp(0.0, 100.0).round() as u32),
                unlocked_at: item.unlocked_at,
                requirements: item
                    .requirements
                    .into_iter()
                    .map(|requirement| XboxAchievementRequirement {
                        id: requirement.id,
                        current: requirement.current,
                        target: requirement.target,
                    })
                    .collect(),
            })
            .collect(),
        continuation: page.continuation,
    })
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_player_playtimes(
    web_token_json: String,
    target_xuid: Option<String>,
    title_ids: Vec<String>,
) -> Result<XboxPlayerPlaytimes, XboxBridgeError> {
    if title_ids.len() > 100 {
        return Err(XboxBridgeError::InvalidData(
            "playtime query supports at most 100 titles".into(),
        ));
    }
    let mut seen = HashSet::new();
    let mut ids = Vec::new();
    for id in title_ids {
        let id = numeric_identity(&id)?;
        if seen.insert(id.clone()) {
            ids.push(id);
        }
    }
    let claims = resolve_web_token_claims(&deserialize::<Value>(&web_token_json)?)?;
    let target = player_identity(target_xuid.as_deref(), claims.xuid.as_deref())?;
    let api = UserStatsApi::new(claims.uhs, claims.token);
    let mut batches = ids
        .chunks(USERSTATS_MAX_TITLES_PER_REQUEST)
        .map(<[String]>::to_vec);
    let mut tasks = JoinSet::new();
    let mut values = HashMap::new();
    let mut failed = HashSet::new();
    let mut failures = Vec::new();
    loop {
        while tasks.len() < 4 {
            let Some(batch) = batches.next() else { break };
            let api = api.clone();
            let target = target.clone();
            tasks.spawn(async move {
                let response = api
                    .get_minutes_played_batch(&target, &batch)
                    .await
                    .and_then(|value| {
                        validate_minutes_played_response(&value)?;
                        Ok(value)
                    });
                (batch, response)
            });
        }
        let Some(result) = tasks.join_next().await else {
            break;
        };
        let (batch, response) =
            result.map_err(|_| XboxBridgeError::Data("游玩时长查询中断".into()))?;
        match response {
            Ok(response) => {
                for id in batch {
                    values.insert(id.clone(), parse_minutes_played(&response, &id));
                }
            }
            Err(error) if error.to_status_code() == Some(401) => return Err(data_error(error)),
            Err(error) => {
                let failure = crate::XboxQueryFailure::from_web_api(&error);
                failures.extend(batch.iter().map(|title| crate::XboxSourceFailure {
                    source: title.clone(),
                    failure: failure.clone(),
                }));
                failed.extend(batch);
            }
        }
    }
    Ok(XboxPlayerPlaytimes {
        items: ids
            .iter()
            .map(|id| XboxPlaytime {
                title_id: id.clone(),
                minutes: values.get(id).copied().flatten(),
            })
            .collect(),
        failed_title_ids: ids.into_iter().filter(|id| failed.contains(id)).collect(),
        failures,
    })
}

fn map_game(game: LibraryGame) -> XboxLibraryGame {
    XboxLibraryGame {
        title_id: game.xbox_title_id,
        product_id: game.product_id,
        scid: game.scid,
        name: game.name,
        description: game.description,
        artwork_url: game.artwork_url,
        last_played_at: game.last_played_at,
        achievement_progress: game.progress.and_then(|progress| {
            let earned = progress.earned_gamerscore?;
            let total = progress.total_gamerscore?;
            Some(XboxAchievementProgress {
                unlocked_count: progress.unlocked_count?,
                total_count: progress.total_count?,
                earned_gamerscore: earned,
                total_gamerscore: total,
                percentage: if total > 0 {
                    (u64::from(earned) * 100 / u64::from(total)).min(100) as u32
                } else {
                    0
                },
            })
        }),
    }
}

fn parse_player_profile(response: &Value, target: &str) -> Result<XboxFriend, XboxBridgeError> {
    let user = library_root(response)
        .get("profileUsers")
        .and_then(Value::as_array)
        .and_then(|users| {
            users
                .iter()
                .find(|user| library_string(user.get("id")).as_deref() == Some(target))
        })
        .ok_or_else(|| XboxBridgeError::Data("玩家资料响应身份无效".into()))?;
    let settings = user
        .get("settings")
        .and_then(Value::as_array)
        .ok_or_else(|| XboxBridgeError::Data("玩家资料格式无效".into()))?;
    let value = |key: &str| {
        settings
            .iter()
            .find(|setting| setting.get("id").and_then(Value::as_str) == Some(key))
            .and_then(|setting| library_string(setting.get("value")))
    };
    Ok(XboxFriend {
        id: target.into(),
        display_name: value("GameDisplayName").unwrap_or_default(),
        gamertag: value("Gamertag").unwrap_or_default(),
        avatar_url: value("GameDisplayPicRaw"),
        gamerscore: value("Gamerscore"),
        presence_state: None,
        presence_text: None,
        current_title_id: None,
    })
}

fn player_identity(target: Option<&str>, account: Option<&str>) -> Result<String, XboxBridgeError> {
    numeric_identity(
        target
            .filter(|value| !value.trim().is_empty())
            .or(account)
            .ok_or_else(|| XboxBridgeError::InvalidData("missing account identity".into()))?,
    )
}

fn numeric_identity(value: &str) -> Result<String, XboxBridgeError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(XboxBridgeError::InvalidData(
            "invalid Xbox numeric identity".into(),
        ));
    }
    Ok(value.into())
}

fn validate_continuation(value: Option<&str>) -> Result<(), XboxBridgeError> {
    if value.is_some_and(|value| {
        value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control)
    }) {
        return Err(XboxBridgeError::InvalidData(
            "invalid pagination continuation".into(),
        ));
    }
    Ok(())
}

fn data_error(error: WebApiError) -> XboxBridgeError {
    crate::query_failure::query_error(error)
}
