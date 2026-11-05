use crate::mods::data::services::xcloud_service::XcloudService;
use crate::mods::data::session_resolver::{resolve_web_token_claims, WebTokenClaims};
use crate::mods::data::types::*;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use xbox_cloud_catalog_flow::{hydrate_catalog_products, CloudCatalogBaseEntry};
use xbox_webapi::{
    library_root, library_string, parse_achievement_page, parse_console_storage,
    parse_game_history, parse_installed_games, parse_minutes_played, parse_people, AchievementsApi,
    GamePassApi, MediaApi, PeopleApi, ProfileApi, SmartglassApi, TitleHubApi, UserPresenceApi,
    UserStatsApi, WebApiError,
};

pub struct PersonalService;
pub struct PersonalContext {
    pub claims: WebTokenClaims,
    pub account_id: String,
    pub target_id: String,
}
impl PersonalContext {
    pub fn wrap<T>(&self, data: T) -> DataQuery<T> {
        DataQuery {
            account_id: self.account_id.clone(),
            target_id: self.target_id.clone(),
            updated_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            data,
        }
    }
}
impl PersonalService {
    pub async fn context(
        session: &DataSessionContext,
        target: Option<&str>,
    ) -> Result<PersonalContext, String> {
        let claims =
            resolve_web_token_claims(&session.web_token).ok_or("登录信息暂不可用，请重新登录")?;
        let account_id = if let Some(id) = &claims.xid {
            id.clone()
        } else {
            let v = request(
                ProfileApi::new(claims.uhs.clone(), claims.user_token.clone()).get_current_user(),
            )
            .await?;
            library_string(
                library_root(&v)
                    .get("profileUsers")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first())
                    .and_then(|p| p.get("id")),
            )
            .ok_or("账户标识暂不可用")?
        };
        let target_id = target
            .filter(|s| !s.is_empty())
            .unwrap_or(&account_id)
            .to_string();
        if !target_id.bytes().all(|b| b.is_ascii_digit()) {
            return Err("玩家标识无效".into());
        }
        Ok(PersonalContext {
            claims,
            account_id,
            target_id,
        })
    }
    pub async fn account(ctx: &PersonalContext) -> DataPersonalAccount {
        let c = &ctx.claims;
        let profile_api = ProfileApi::new(c.uhs.clone(), c.user_token.clone());
        let people_api = PeopleApi::new(c.uhs.clone(), c.user_token.clone());
        let presence_api = UserPresenceApi::new(c.uhs.clone(), c.user_token.clone());
        let own = ctx.target_id == ctx.account_id;
        let (profile, friends, presence) = tokio::join!(
            request(profile_api.get_user(&ctx.target_id)),
            async {
                if own {
                    request(people_api.get_friends())
                        .await
                        .and_then(|v| parse_people(&v).map_err(user_error))
                } else {
                    Ok(Vec::new())
                }
            },
            async {
                if own {
                    request(presence_api.get_current_user()).await.map(Some)
                } else {
                    Ok(None)
                }
            }
        );
        let mut notices = Vec::new();
        let mut person = match profile {
            Ok(v) => parse_profile(&v, &ctx.target_id),
            Err(e) => {
                notices.push(format!("资料：{e}"));
                None
            }
        };
        if let Some(person) = &mut person {
            match presence {
                Ok(Some(p)) => {
                    person.presence_state = p.state;
                    person.presence_text = p.current_title_name.or(p.rich_presence);
                }
                Err(e) => notices.push(format!("在线状态：{e}")),
                _ => {}
            }
        }
        let friends = match friends {
            Ok(v) => own.then_some(v),
            Err(e) => {
                notices.push(format!("好友：{e}"));
                None
            }
        };
        DataPersonalAccount {
            profile: person,
            friends,
            notices,
        }
    }
    pub async fn game(ctx: &PersonalContext, title: &str) -> Result<Option<LibraryGame>, String> {
        let api = TitleHubApi::new(ctx.claims.uhs.clone(), ctx.claims.user_token.clone());
        let v = request(api.get_title_detail(&ctx.target_id, title)).await?;
        Ok(parse_game_history(&v)
            .map_err(user_error)?
            .items
            .into_iter()
            .next())
    }
    pub async fn history(
        ctx: &PersonalContext,
        continuation: Option<&str>,
    ) -> Result<LibraryPage<LibraryGame>, String> {
        let api = TitleHubApi::new(ctx.claims.uhs.clone(), ctx.claims.user_token.clone());
        let v = request(api.get_title_history_page(&ctx.target_id, continuation)).await?;
        parse_game_history(&v).map_err(user_error)
    }
    pub async fn achievements(
        ctx: &PersonalContext,
        title: &str,
        continuation: Option<&str>,
        language: &str,
    ) -> Result<LibraryPage<LibraryAchievement>, String> {
        let api = AchievementsApi::new(
            ctx.claims.uhs.clone(),
            ctx.claims.user_token.clone(),
            language.to_string(),
        );
        let v = request(api.get_title_achievements(&ctx.target_id, title, continuation, Some(200)))
            .await?;
        parse_achievement_page(&v).map_err(user_error)
    }
    pub async fn playtimes(
        ctx: &PersonalContext,
        ids: Vec<String>,
    ) -> Result<Vec<DataPlaytime>, String> {
        let mut seen = HashSet::new();
        let ids: Vec<_> = ids
            .into_iter()
            .filter(|id| seen.insert(id.clone()))
            .collect();
        if ids.len() > 100
            || ids
                .iter()
                .any(|id| id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err("每次支持至多 100 个有效游戏标识".into());
        }
        let api = UserStatsApi::new(ctx.claims.uhs.clone(), ctx.claims.user_token.clone());
        let mut output = Vec::new();
        // 统计请求固定四路并发，单项失败保留其余游戏数据。
        for batch in ids.chunks(4) {
            let mut tasks = tokio::task::JoinSet::new();
            for id in batch {
                let id = id.clone();
                let api = api.clone();
                let target = ctx.target_id.clone();
                tasks.spawn(async move {
                    match request(api.get_minutes_played(&target, &id)).await {
                        Ok(v) => DataPlaytime {
                            minutes: parse_minutes_played(&v, &id),
                            xbox_title_id: id,
                            error: None,
                        },
                        Err(e) => DataPlaytime {
                            xbox_title_id: id,
                            minutes: None,
                            error: Some(e),
                        },
                    }
                });
            }
            while let Some(result) = tasks.join_next().await {
                output.push(result.map_err(|_| "时长查询已中断")?);
            }
        }
        Ok(output)
    }
    pub async fn console(ctx: &PersonalContext, id: &str) -> Result<DataConsoleLibrary, String> {
        if id.trim().is_empty() {
            return Err("主机标识暂不可用".into());
        }
        let api = SmartglassApi::new(ctx.claims.uhs.clone(), ctx.claims.user_token.clone());
        let (games, storage, status) = tokio::join!(
            request(api.get_installed_apps(id)),
            request(api.get_storage_devices(id)),
            request(api.get_console_status(id))
        );
        let mut notices = Vec::new();
        let mut games = match games.and_then(|v| parse_installed_games(&v).map_err(user_error)) {
            Ok(v) => Some(v),
            Err(e) => {
                notices.push(format!("安装游戏：{e}"));
                None
            }
        };
        let storage = match storage.and_then(|v| parse_console_storage(&v).map_err(user_error)) {
            Ok(v) => Some(v),
            Err(e) => {
                notices.push(format!("存储：{e}"));
                None
            }
        };
        let power_state = match status {
            Ok(v) => library_string(library_root(&v).get("powerState")),
            Err(e) => {
                notices.push(format!("主机状态：{e}"));
                None
            }
        };
        if let Some(games) = &mut games {
            let ids: Vec<_> = games.iter().filter_map(|g| g.product_id.clone()).collect();
            if !ids.is_empty() {
                let scope = XcloudService::resolve_catalog_scope();
                let api = GamePassApi::new();
                match tokio::time::timeout(
                    std::time::Duration::from_secs(8),
                    hydrate_catalog_products(&api, &scope, &ids),
                )
                .await
                {
                    Ok(load) => {
                        apply_installed_artwork(games, &load.entries);
                        if load.failed_chunk_count > 0 {
                            notices.push("部分游戏封面暂不可用，安装信息已保留".into());
                        }
                    }
                    Err(_) => notices.push("游戏封面查询超时，安装信息已保留".into()),
                }
            }
        }
        Ok(DataConsoleLibrary {
            console_id: id.to_string(),
            games,
            storage,
            power_state,
            notices,
        })
    }
    pub async fn media(
        ctx: &PersonalContext,
        kind: MediaKind,
        continuation: Option<&str>,
    ) -> Result<LibraryPage<LibraryMedia>, String> {
        request(
            MediaApi::new(ctx.claims.uhs.clone(), ctx.claims.user_token.clone()).search(
                &ctx.target_id,
                kind,
                continuation,
            ),
        )
        .await
    }
}
fn apply_installed_artwork(
    games: &mut [InstalledGame],
    products: &HashMap<String, CloudCatalogBaseEntry>,
) {
    for game in games {
        let Some(product) = game
            .product_id
            .as_ref()
            .and_then(|id| products.get(&id.trim().to_ascii_uppercase()))
        else {
            continue;
        };
        game.artwork_url = [
            product.tile_image_url.as_str(),
            product.poster_image_url.as_str(),
        ]
        .into_iter()
        .find(|url| !url.is_empty())
        .map(str::to_string);
    }
}
async fn request<T>(
    future: impl std::future::Future<Output = Result<T, WebApiError>>,
) -> Result<T, String> {
    tokio::time::timeout(std::time::Duration::from_secs(20), future)
        .await
        .map_err(|_| "查询超时，请稍后刷新".to_string())?
        .map_err(user_error)
}
fn user_error(e: WebApiError) -> String {
    match e.to_status_code() {
        Some(401) => "登录已过期，请重新登录".into(),
        Some(403) => "该玩家的隐私设置或账户权限限制了此项数据".into(),
        Some(429) => "Xbox 服务请求频繁，请稍后刷新".into(),
        _ => "Xbox 服务暂不可用，请稍后刷新".into(),
    }
}
fn parse_profile(v: &Value, xuid: &str) -> Option<LibraryPerson> {
    let user = library_root(v).get("profileUsers")?.as_array()?.first()?;
    let settings: HashMap<_, _> = user
        .get("settings")?
        .as_array()?
        .iter()
        .filter_map(|s| Some((s.get("id")?.as_str()?, library_string(s.get("value"))?)))
        .collect();
    Some(LibraryPerson {
        xuid: xuid.to_string(),
        display_name: settings.get("GameDisplayName").cloned().unwrap_or_default(),
        gamertag: settings.get("Gamertag").cloned().unwrap_or_default(),
        avatar_url: settings.get("GameDisplayPicRaw").cloned(),
        gamerscore: settings.get("Gamerscore").cloned(),
        presence_state: None,
        presence_text: None,
        current_title_id: None,
        is_followed_by_caller: None,
        is_following_caller: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn installed_artwork_matches_product_identity_and_preserves_missing_metadata() {
        let mut games = parse_installed_games(&json!({"result":[
            {"instanceId":"one","isGame":true,"name":"One","oneStoreProductId":"product-a","titleId":99},
            {"instanceId":"two","isGame":true,"name":"Two","oneStoreProductId":"PRODUCT-B","titleId":100},
            {"instanceId":"three","isGame":true,"name":"Three","titleId":101}
        ]})).unwrap();
        let products = HashMap::from([(
            "PRODUCT-A".into(),
            CloudCatalogBaseEntry {
                tile_image_url: "https://example.com/tile.jpg".into(),
                poster_image_url: "https://example.com/poster.jpg".into(),
                ..Default::default()
            },
        )]);
        apply_installed_artwork(&mut games, &products);
        assert_eq!(
            games[0].artwork_url.as_deref(),
            Some("https://example.com/tile.jpg")
        );
        assert!(games[1].artwork_url.is_none());
        assert!(games[2].artwork_url.is_none());
        assert_eq!(games[1].name, "Two");
    }
}
