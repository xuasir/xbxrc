use crate::mods::data::session_resolver::{resolve_web_token_claims, WebTokenClaims};
use crate::mods::data::types::*;
use crate::mods::runtime_trace::RuntimeTraceRecorderRef;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Semaphore;
use xbox_cloud_catalog_flow::{
    hydrate_catalog_products_with_budget, CloudCatalogBaseEntry, CloudCatalogScope,
};
use xbox_webapi::{
    library_image_url, library_root, library_string, parse_achievement_page, parse_console_storage,
    parse_console_storage_for_device, parse_game_history, parse_installed_games,
    parse_minutes_played, parse_people, validate_minutes_played_response, AchievementsApi,
    GamePassApi, MediaApi, PeopleApi, ProfileApi, SmartglassApi, TitleHubApi, UserPresenceApi,
    UserStatsApi, WebApiError, USERSTATS_MAX_TITLES_PER_REQUEST,
};

pub struct PersonalService;
#[derive(Clone)]
pub struct PersonalContext {
    pub claims: WebTokenClaims,
    pub account_id: String,
    pub target_id: String,
    runtime_trace: RuntimeTraceRecorderRef,
}
impl PersonalContext {
    async fn request<T>(
        &self,
        operation: &'static str,
        future: impl std::future::Future<Output = Result<T, WebApiError>>,
    ) -> Result<T, WebApiError> {
        request(&self.runtime_trace, operation, future).await
    }

    fn record_parsed(&self, operation: &'static str, counts: Value) {
        self.runtime_trace.record_event(
            "data",
            "httpResponseParsed",
            None,
            serde_json::json!({ "operation": operation, "counts": counts }),
        );
    }

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
        runtime_trace: RuntimeTraceRecorderRef,
    ) -> Result<PersonalContext, WebApiError> {
        let claims = resolve_web_token_claims(&session.web_token)
            .ok_or_else(|| WebApiError::auth("登录信息暂不可用，请重新登录"))?;
        let account_id = if let Some(id) = &claims.xid {
            id.clone()
        } else {
            let v = request(
                &runtime_trace,
                "accountIdentifier",
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
            .ok_or_else(|| WebApiError::parse("账户标识暂不可用"))?
        };
        let target_id = target
            .filter(|s| !s.is_empty())
            .unwrap_or(&account_id)
            .to_string();
        if !target_id.bytes().all(|b| b.is_ascii_digit()) {
            return Err(WebApiError::parse("玩家标识无效"));
        }
        Ok(PersonalContext {
            claims,
            account_id,
            target_id,
            runtime_trace,
        })
    }
    pub async fn account(ctx: &PersonalContext) -> Result<DataPersonalAccount, WebApiError> {
        let c = &ctx.claims;
        let profile_api = ProfileApi::new(c.uhs.clone(), c.user_token.clone());
        let people_api = PeopleApi::new(c.uhs.clone(), c.user_token.clone());
        let presence_api = UserPresenceApi::new(c.uhs.clone(), c.user_token.clone());
        let own = ctx.target_id == ctx.account_id;
        let (profile, friends, presence) = tokio::join!(
            ctx.request("accountProfile", profile_api.get_user(&ctx.target_id)),
            async {
                if own {
                    ctx.request("friends", people_api.get_friends())
                        .await
                        .and_then(|v| parse_people(&v))
                } else {
                    Ok(Vec::new())
                }
            },
            async {
                if own {
                    ctx.request("presence", presence_api.get_current_user())
                        .await
                        .map(Some)
                } else {
                    Ok(None)
                }
            }
        );
        let profile = profile.and_then(|value| {
            parse_profile(&value, &ctx.target_id)
                .ok_or_else(|| WebApiError::parse("invalid personal profile response"))
        });
        if let Some(error) = [
            profile.as_ref().err(),
            friends.as_ref().err(),
            presence.as_ref().err(),
        ]
        .into_iter()
        .flatten()
        .find(|error| error.to_status_code() == Some(401))
        {
            return Err(error.clone());
        }
        let mut notices = Vec::new();
        let mut person = match profile {
            Ok(v) => Some(v),
            Err(e) => {
                notices.push(format!(
                    "资料：{}",
                    partial_query_error(ctx, "accountProfile", &e)
                ));
                None
            }
        };
        if let Some(person) = &mut person {
            match presence {
                Ok(Some(p)) => {
                    person.presence_state = p.state;
                    person.presence_text = p.current_title_name.or(p.rich_presence);
                }
                Err(e) => notices.push(format!(
                    "在线状态：{}",
                    partial_query_error(ctx, "presence", &e)
                )),
                _ => {}
            }
        }
        let friends = match friends {
            Ok(v) => own.then_some(v),
            Err(e) => {
                notices.push(format!("好友：{}", partial_query_error(ctx, "friends", &e)));
                None
            }
        };
        ctx.record_parsed(
            "personalAccount",
            serde_json::json!({
                "profileAvailable": person.is_some(),
                "friendsAvailable": friends.is_some(),
                "friendCount": friends.as_ref().map(Vec::len),
                "noticeCount": notices.len(),
            }),
        );
        Ok(DataPersonalAccount {
            profile: person,
            friends,
            notices,
        })
    }
    pub async fn game(
        ctx: &PersonalContext,
        title: &str,
    ) -> Result<Option<LibraryGame>, WebApiError> {
        let api = TitleHubApi::new(ctx.claims.uhs.clone(), ctx.claims.user_token.clone());
        let v = ctx
            .request("titleDetail", api.get_title_detail(&ctx.target_id, title))
            .await?;
        Ok(parse_game_history(&v)?.items.into_iter().next())
    }
    pub async fn history(
        ctx: &PersonalContext,
        continuation: Option<&str>,
    ) -> Result<LibraryPage<LibraryGame>, WebApiError> {
        let api = TitleHubApi::new(ctx.claims.uhs.clone(), ctx.claims.user_token.clone());
        let v = ctx
            .request(
                "titleHistory",
                api.get_title_history_page(&ctx.target_id, continuation),
            )
            .await?;
        let page = parse_game_history(&v)?;
        ctx.record_parsed("titleHistory", serde_json::json!({
            "itemCount": page.items.len(),
            "artworkCount": page.items.iter().filter(|game| game.artwork_url.is_some()).count(),
            "achievementSummaryCount": page.items.iter().filter(|game| game.progress.is_some()).count(),
            "hasContinuation": page.continuation.is_some(),
        }));
        Ok(page)
    }
    pub async fn achievements(
        ctx: &PersonalContext,
        title: &str,
        continuation: Option<&str>,
        language: &str,
    ) -> Result<LibraryPage<LibraryAchievement>, WebApiError> {
        let api = AchievementsApi::new(
            ctx.claims.uhs.clone(),
            ctx.claims.user_token.clone(),
            language.to_string(),
        );
        let v = ctx
            .request(
                "achievements",
                api.get_title_achievements(&ctx.target_id, title, continuation, Some(200)),
            )
            .await?;
        let page = parse_achievement_page(&v)?;
        ctx.record_parsed("achievements", serde_json::json!({
            "itemCount": page.items.len(),
            "unlockedCount": page.items.iter().filter(|achievement| achievement.is_unlocked).count(),
            "hasContinuation": page.continuation.is_some(),
        }));
        Ok(page)
    }
    pub async fn playtimes(
        ctx: &PersonalContext,
        ids: Vec<String>,
    ) -> Result<Vec<DataPlaytime>, WebApiError> {
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
            return Err(WebApiError::parse("每次支持至多 100 个有效游戏标识"));
        }
        let api = UserStatsApi::new(ctx.claims.uhs.clone(), ctx.claims.user_token.clone());
        let mut output = Vec::new();
        let permits = Arc::new(Semaphore::new(4));
        let mut tasks = tokio::task::JoinSet::new();
        // 服务支持多标题 /batch；固定四路并发，批次失败保留其他批次结果。
        for batch in ids.chunks(USERSTATS_MAX_TITLES_PER_REQUEST) {
            let ids = batch.to_vec();
            let api = api.clone();
            let ctx = ctx.clone();
            let permits = permits.clone();
            tasks.spawn(async move {
                let _permit = permits
                    .acquire_owned()
                    .await
                    .map_err(|_| WebApiError::network("时长查询已中断"))?;
                let response = ctx
                    .request(
                        "playtimeBatch",
                        api.get_minutes_played_batch(&ctx.target_id, &ids),
                    )
                    .await;
                playtime_batch_result(&ctx, ids, response)
            });
        }
        while let Some(result) = tasks.join_next().await {
            output.extend(result.map_err(|_| WebApiError::network("时长查询已中断"))??);
        }
        ctx.record_parsed(
            "playtimes",
            serde_json::json!({
                "requestedCount": ids.len(),
                "availableCount": output.iter().filter(|item| item.minutes.is_some()).count(),
                "failedCount": output.iter().filter(|item| item.error.is_some()).count(),
            }),
        );
        Ok(output)
    }
    pub async fn console(
        ctx: &PersonalContext,
        id: &str,
        scope: &CloudCatalogScope,
    ) -> Result<DataConsoleLibrary, WebApiError> {
        if id.trim().is_empty() {
            return Err(WebApiError::parse("主机标识暂不可用"));
        }
        let api = SmartglassApi::new(ctx.claims.uhs.clone(), ctx.claims.user_token.clone());
        let (games, storage, status) = tokio::join!(
            ctx.request("installedGames", api.get_installed_apps(id)),
            ctx.request("consoleStorage", api.get_storage_devices(id)),
            ctx.request("consoleStatus", api.get_console_status(id))
        );
        if let Some(error) = [
            games.as_ref().err(),
            storage.as_ref().err(),
            status.as_ref().err(),
        ]
        .into_iter()
        .flatten()
        .find(|error| error.to_status_code() == Some(401))
        {
            return Err(error.clone());
        }
        let mut notices = Vec::new();
        let mut games = match games.and_then(|v| parse_installed_games(&v)) {
            Ok(v) => Some(v),
            Err(e) => {
                notices.push(format!(
                    "安装游戏：{}",
                    partial_query_error(ctx, "installedGames", &e)
                ));
                None
            }
        };
        let storage = match resolve_console_storage(ctx, id, storage, api.get_consoles_list()).await
        {
            Ok(v) => Some(v),
            Err(e) if e.to_status_code() == Some(401) => return Err(e),
            Err(e) => {
                notices.push(format!(
                    "存储：{}",
                    partial_query_error(ctx, "consoleStorage", &e)
                ));
                None
            }
        };
        let power_state = match status.and_then(|value| parse_console_power_state(&value)) {
            Ok(v) => Some(v),
            Err(e) => {
                notices.push(format!(
                    "主机状态：{}",
                    partial_query_error(ctx, "consoleStatus", &e)
                ));
                None
            }
        };
        if let Some(games) = &mut games {
            let ids: Vec<_> = games.iter().filter_map(|g| g.product_id.clone()).collect();
            if !ids.is_empty() {
                let api = GamePassApi::new();
                let load = hydrate_catalog_products_with_budget(
                    &api,
                    scope,
                    &ids,
                    std::time::Duration::from_secs(8),
                )
                .await;
                apply_installed_artwork(games, &load.entries);
                if load.failed_chunk_count > 0 {
                    notices.push("部分游戏封面暂不可用，安装信息已保留".into());
                    for failure in &load.source_failures {
                        log::warn!(
                            "[Data][consoleLibrary] source={} error={}",
                            failure.source,
                            failure.error
                        );
                    }
                }
            }
        }
        ctx.record_parsed(
            "consoleLibrary",
            serde_json::json!({
                "gamesAvailable": games.is_some(),
                "gameCount": games.as_ref().map(Vec::len),
                "storageAvailable": storage.is_some(),
                "storageCount": storage.as_ref().map(Vec::len),
                "noticeCount": notices.len(),
            }),
        );
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
    ) -> Result<LibraryPage<LibraryMedia>, WebApiError> {
        let page = ctx
            .request(
                "mediaSearch",
                MediaApi::new(ctx.claims.uhs.clone(), ctx.claims.user_token.clone()).search(
                    &ctx.target_id,
                    kind,
                    continuation,
                ),
            )
            .await?;
        ctx.record_parsed("mediaSearch", serde_json::json!({
            "itemCount": page.items.len(),
            "thumbnailCount": page.items.iter().filter(|media| media.thumbnail_url.is_some()).count(),
            "hasContinuation": page.continuation.is_some(),
        }));
        Ok(page)
    }
}

async fn resolve_console_storage(
    ctx: &PersonalContext,
    console_id: &str,
    response: Result<Value, WebApiError>,
    summary: impl std::future::Future<Output = Result<Value, WebApiError>>,
) -> Result<Vec<ConsoleStorage>, WebApiError> {
    let storage = response.and_then(|value| parse_console_storage(&value))?;
    if !storage.is_empty() {
        return Ok(storage);
    }
    // XCCS 独立存储查询可返回空列表；设备列表的嵌入摘要按身份提供实际磁盘容量。
    let payload = ctx.request("consoleStorageSummary", summary).await?;
    parse_console_storage_for_device(&payload, console_id)
}

fn playtime_batch_result(
    ctx: &PersonalContext,
    ids: Vec<String>,
    response: Result<Value, WebApiError>,
) -> Result<Vec<DataPlaytime>, WebApiError> {
    let response = response.and_then(|value| {
        validate_minutes_played_response(&value)?;
        Ok(value)
    });
    let output: Vec<_> = match response {
        Ok(value) => ids
            .into_iter()
            .map(|id| DataPlaytime {
                minutes: parse_minutes_played(&value, &id),
                xbox_title_id: id,
                error: None,
            })
            .collect(),
        Err(error) if error.to_status_code() == Some(401) => return Err(error),
        Err(error) => {
            let message = partial_query_error(ctx, "playtimeBatch", &error);
            ids.into_iter()
                .map(|id| DataPlaytime {
                    xbox_title_id: id,
                    minutes: None,
                    error: Some(message.clone()),
                })
                .collect()
        }
    };
    ctx.record_parsed("playtimeBatch", serde_json::json!({
        "requestedCount": output.len(),
        "availableCount": output.iter().filter(|item| item.minutes.is_some()).count(),
        "missingCount": output.iter().filter(|item| item.minutes.is_none() && item.error.is_none()).count(),
        "failedCount": output.iter().filter(|item| item.error.is_some()).count(),
    }));
    Ok(output)
}

fn partial_query_error(
    ctx: &PersonalContext,
    operation: &'static str,
    error: &WebApiError,
) -> String {
    ctx.runtime_trace.record_event(
        "data",
        "httpPartialQueryFailed",
        None,
        serde_json::json!({
            "operation": operation,
            "error": error.safe_summary(),
        }),
    );
    log::warn!(
        "[Data][http] operation={} outcome=partial error={}",
        operation,
        error.safe_summary()
    );
    user_error(error)
}

fn parse_console_power_state(value: &Value) -> Result<String, WebApiError> {
    library_string(library_root(value).get("powerState"))
        .ok_or_else(|| WebApiError::parse("invalid console status response"))
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
    recorder: &RuntimeTraceRecorderRef,
    operation: &'static str,
    future: impl std::future::Future<Output = Result<T, WebApiError>>,
) -> Result<T, WebApiError> {
    request_with_budget(
        recorder,
        operation,
        future,
        std::time::Duration::from_secs(20),
    )
    .await
}

async fn request_with_budget<T>(
    recorder: &RuntimeTraceRecorderRef,
    operation: &'static str,
    future: impl std::future::Future<Output = Result<T, WebApiError>>,
    budget: std::time::Duration,
) -> Result<T, WebApiError> {
    let started = std::time::Instant::now();
    let result = tokio::time::timeout(budget, future)
        .await
        .unwrap_or_else(|_| {
            Err(WebApiError::network_kind(
                xbox_webapi::NetworkErrorKind::Timeout,
            ))
        });
    recorder.record_event(
        "data",
        "httpSourceFinished",
        None,
        serde_json::json!({
            "operation": operation,
            "elapsedMs": started.elapsed().as_millis(),
            "outcome": if result.is_ok() { "success" } else { "failed" },
            "error": result.as_ref().err().map(WebApiError::safe_summary),
        }),
    );
    result
}
pub(crate) fn user_error(e: &WebApiError) -> String {
    if let WebApiError::Auth { message } = e {
        return message.clone();
    }
    if let WebApiError::Parse { message } = e {
        return match message.as_str() {
            "玩家标识无效"
            | "每次支持至多 100 个有效游戏标识"
            | "主机标识暂不可用"
            | "账户标识暂不可用"
            | "资源已移除，请刷新相册"
            | "目录缓存读取失败，请重试"
            | "目录缓存写入失败，请检查存储空间"
            | "登录状态已更新，请重新查询"
            | "登录或目录设置已更新，请刷新"
            | "主机远程管理查询暂不可用" => message.clone(),
            _ => "Xbox 服务响应异常，请稍后刷新".into(),
        };
    }
    if let Some(kind) = e.network_error_kind() {
        return match kind {
            xbox_webapi::NetworkErrorKind::Dns => "Xbox 服务域名解析失败，请检查网络或 DNS".into(),
            xbox_webapi::NetworkErrorKind::Tls => {
                "Xbox 服务安全连接失败，请检查系统时间和网络".into()
            }
            xbox_webapi::NetworkErrorKind::Timeout => "查询超时，请稍后刷新".into(),
            _ => "Xbox 服务连接失败，请检查网络后刷新".into(),
        };
    }
    match e.to_status_code() {
        Some(401) => "登录已过期，请重新登录".into(),
        Some(403) => match e.safe_service_code().as_deref() {
            Some(
                "UnsupportedCountry" | "UnsupportedRegion" | "BlockedRegion" | "RegionNotSupported",
            ) => "Xbox 服务限制了当前地区的访问".into(),
            Some("SubscriptionRequired" | "NoValidSubscription" | "OfferingAccessDenied") => {
                "当前账户缺少此项服务所需资格".into()
            }
            _ => "该玩家的隐私设置或账户权限限制了此项数据".into(),
        },
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
    if settings.is_empty() {
        return None;
    }
    Some(LibraryPerson {
        xuid: xuid.to_string(),
        display_name: settings.get("GameDisplayName").cloned().unwrap_or_default(),
        gamertag: settings.get("Gamertag").cloned().unwrap_or_default(),
        avatar_url: settings
            .get("GameDisplayPicRaw")
            .and_then(|url| library_image_url(Some(&Value::String(url.clone())))),
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

    fn context_for_test(recorder: RuntimeTraceRecorderRef) -> PersonalContext {
        PersonalContext {
            claims: WebTokenClaims {
                uhs: "fixture-secret-uhs".into(),
                user_token: "fixture-secret-token".into(),
                xid: Some("private-account".into()),
            },
            account_id: "private-account".into(),
            target_id: "private-target".into(),
            runtime_trace: recorder,
        }
    }

    #[tokio::test]
    async fn empty_storage_query_uses_matching_device_summary_and_promotes_auth_recovery() {
        let recorder: RuntimeTraceRecorderRef = Arc::new(
            crate::mods::runtime_trace::RuntimeTraceRecorder::new_with_mode("dev").unwrap(),
        );
        let ctx = context_for_test(recorder);
        let primary = || Ok(json!({"status":{"errorCode":"OK"},"result":[]}));
        let storage = resolve_console_storage(&ctx, "console", primary(), async {
            Ok(json!({"status":{"errorCode":"OK"},"result":[
                {"id":"other","storageDevices":[{"storageDeviceId":"other-disk"}]},
                {"id":"console","storageDevices":[{"storageDeviceId":"disk","freeSpaceBytes":42.0,"totalSpaceBytes":100.0}]}
            ]}))
        }).await.unwrap();
        assert_eq!(storage.len(), 1);
        assert_eq!(storage[0].id, "disk");
        assert_eq!(storage[0].free_bytes, Some(42));
        assert_eq!(storage[0].total_bytes, Some(100));
        assert_eq!(
            resolve_console_storage(&ctx, "console", primary(), async {
                Err(WebApiError::http(401, "fixture-secret-body"))
            })
            .await
            .unwrap_err()
            .to_status_code(),
            Some(401)
        );
    }

    #[tokio::test]
    async fn available_or_failed_storage_query_keeps_its_original_result() {
        let recorder: RuntimeTraceRecorderRef = Arc::new(
            crate::mods::runtime_trace::RuntimeTraceRecorder::new_with_mode("dev").unwrap(),
        );
        let ctx = context_for_test(recorder);
        let storage = resolve_console_storage(
            &ctx,
            "console",
            Ok(json!({"result":[{"storageDeviceId":"disk"}]})),
            async { panic!("available storage should complete without an additional query") },
        )
        .await
        .unwrap();
        assert_eq!(storage[0].id, "disk");
        assert_eq!(
            resolve_console_storage(
                &ctx,
                "console",
                Err(WebApiError::http(403, "fixture-secret-body")),
                async { panic!("service rejection should retain the original error") }
            )
            .await
            .unwrap_err()
            .to_status_code(),
            Some(403)
        );
    }

    #[test]
    fn batch_playtimes_preserve_each_title_and_promote_auth_recovery() {
        let recorder: RuntimeTraceRecorderRef = Arc::new(
            crate::mods::runtime_trace::RuntimeTraceRecorder::new_with_mode("dev").unwrap(),
        );
        let ctx = context_for_test(recorder);
        let ids = vec!["4294967295".into(), "1292135258".into(), "100".into()];
        let rows = playtime_batch_result(
            &ctx,
            ids.clone(),
            Ok(json!({"statlistscollection":[{"stats":[
                {"titleid":"1292135258","name":"MinutesPlayed","value":"0"},
                {"titleid":"4294967295","name":"MinutesPlayed","value":"79"}
            ]}]})),
        )
        .unwrap();
        assert_eq!(
            rows.iter().map(|row| row.minutes).collect::<Vec<_>>(),
            vec![Some(79), Some(0), None]
        );
        assert!(rows.iter().all(|row| row.error.is_none()));
        let failed = playtime_batch_result(
            &ctx,
            ids.clone(),
            Err(WebApiError::http(429, "fixture-secret-body")),
        )
        .unwrap();
        assert_eq!(failed.len(), 3);
        assert!(failed.iter().all(|row| row
            .error
            .as_deref()
            .is_some_and(|message| message.contains("频繁"))));
        assert_eq!(
            playtime_batch_result(
                &ctx,
                ids,
                Err(WebApiError::http(401, "fixture-secret-body"))
            )
            .unwrap_err()
            .to_status_code(),
            Some(401)
        );
        let invalid = playtime_batch_result(&ctx, vec!["100".into()], Ok(json!({}))).unwrap();
        assert!(invalid[0].error.is_some());
        assert!(invalid[0].minutes.is_none());
    }

    #[tokio::test]
    async fn source_partial_and_parse_diagnostics_survive_both_profiles_safely() {
        for profile in ["production", "dev"] {
            let recorder: RuntimeTraceRecorderRef = Arc::new(
                crate::mods::runtime_trace::RuntimeTraceRecorder::new_with_mode(profile).unwrap(),
            );
            let ctx = context_for_test(recorder.clone());
            let timeout = request_with_budget(
                &recorder,
                "friends",
                std::future::pending::<Result<(), WebApiError>>(),
                std::time::Duration::from_millis(1),
            )
            .await
            .unwrap_err();
            assert_eq!(
                timeout.network_error_kind(),
                Some(xbox_webapi::NetworkErrorKind::Timeout)
            );
            partial_query_error(&ctx, "friends", &WebApiError::parse("missing people array"));
            playtime_batch_result(&ctx, vec!["private-title".into()], Ok(json!({"stats":[]})))
                .unwrap();
            let contents = std::fs::read_to_string(recorder.path().unwrap()).unwrap();
            let rows: Vec<Value> = contents
                .lines()
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .filter(|row| row["domain"] == "data")
                .collect();
            assert_eq!(rows.len(), 3);
            assert!(rows
                .iter()
                .all(|row| row["importance"] == "key" && row["dimension"] == "core"));
            assert_eq!(rows[0]["payload"]["error"]["networkErrorKind"], "timeout");
            assert_eq!(
                rows[1]["payload"]["error"]["protocolErrorCode"],
                "missingPeople"
            );
            assert_eq!(rows[2]["payload"]["counts"]["missingCount"], 1);
            for private in [
                "fixture-secret",
                "private-account",
                "private-target",
                "private-title",
            ] {
                assert!(!contents.contains(private));
            }
        }
    }

    #[test]
    fn validates_personal_profile_and_console_status_schemas() {
        assert!(parse_profile(&json!({}), "1").is_none());
        assert!(parse_profile(&json!({"profileUsers": [{"settings": []}]}), "1").is_none());
        assert_eq!(
            parse_profile(
                &json!({"profileUsers": [{"settings": [{"id":"Gamertag", "value":"Player"}]}]}),
                "1"
            )
            .unwrap()
            .gamertag,
            "Player"
        );
        assert!(parse_console_power_state(&json!({"powerState": null})).is_err());
        assert_eq!(
            parse_console_power_state(&json!({"data": {"powerState": "Connected"}})).unwrap(),
            "Connected"
        );
    }

    #[test]
    fn returns_safe_contextual_http_errors_and_known_cache_guidance() {
        for message in [
            "目录缓存读取失败，请重试",
            "目录缓存写入失败，请检查存储空间",
        ] {
            assert_eq!(user_error(&WebApiError::parse(message)), message);
        }
        let error = WebApiError::http_with_code(403, "UnsupportedCountry", "secret-body");
        assert!(user_error(&error).contains("地区"));
        assert!(!user_error(&error).contains("secret"));
        assert!(user_error(&WebApiError::network_kind(
            xbox_webapi::NetworkErrorKind::Dns
        ))
        .contains("DNS"));
        assert!(user_error(&WebApiError::http(429, "secret-body")).contains("频繁"));
        assert!(!user_error(&WebApiError::parse("secret-body")).contains("secret"));
    }

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
