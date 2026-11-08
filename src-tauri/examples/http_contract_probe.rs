//! 只读接口复核：cargo run -p xbxrc --example http_contract_probe -- <settings.xbx.json>
//! 凭据只保存在进程内；输出限于数量、耗时和固定错误分类。
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};
use xbox_cloud_catalog_flow::CloudCatalogScope;
use xbox_webapi::{AuthApi, MediaKind, WebApiError};
use xbxrc_lib::mods::data::services::personal_service::PersonalService;
use xbxrc_lib::mods::data::services::HostService;
use xbxrc_lib::mods::data::types::DataSessionContext;
use xbxrc_lib::mods::runtime_trace::{
    set_trace_root_dir, RuntimeTraceRecorder, RuntimeTraceRecorderRef,
};

fn emit(operation: &str, facts: Value) {
    println!("{}", json!({"operation": operation, "facts": facts}));
}
fn failure(operation: &str, error: &WebApiError) {
    emit(
        operation,
        json!({"outcome":"failed", "error":error.safe_summary()}),
    );
}

#[tokio::main]
async fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("用法：http_contract_probe <settings.xbx.json>；执行只读查询及云授权交换，输出脱敏汇总。");
        std::process::exit(2);
    };
    let Ok(contents) = std::fs::read_to_string(path) else {
        eprintln!("配置文件无法读取");
        std::process::exit(2);
    };
    let Ok(store) = serde_json::from_str::<Value>(&contents) else {
        eprintln!("配置格式无效");
        std::process::exit(2);
    };
    let token = store["auth.tokens.web"].clone();
    let token_root = token.get("data").unwrap_or(&token);
    let valid = token_root["NotAfter"]
        .as_str()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .is_some_and(|expiry| expiry > chrono::Utc::now() + chrono::Duration::seconds(60));
    if !valid {
        eprintln!("当前 Web 凭据需要刷新");
        std::process::exit(2);
    }
    set_trace_root_dir(std::env::temp_dir().join("xbxrc-http-contract-probe"));
    let recorder: RuntimeTraceRecorderRef =
        Arc::new(RuntimeTraceRecorder::new_with_mode("dev").expect("trace initialization"));
    let session = DataSessionContext {
        provider: "xal".into(),
        app_level: 1,
        web_token: token,
        streaming_tokens: store["auth.tokens.stream"].clone(),
    };
    let ctx = match PersonalService::context(&session, None, recorder.clone()).await {
        Ok(ctx) => ctx,
        Err(error) => {
            failure("context", &error);
            std::process::exit(1);
        }
    };
    let started = Instant::now();
    let host_service = HostService::new();
    let (hosts, account, history) = tokio::join!(
        host_service.get_hosts(&session),
        PersonalService::account(&ctx),
        PersonalService::history(&ctx, None),
    );
    match &hosts {
        Ok(hosts) => emit(
            "hosts",
            json!({"outcome":"success", "count":hosts.len(), "storageCount":hosts.iter().flat_map(|host| host.storage_devices.iter().flatten()).count(), "storageWithCapacity":hosts.iter().flat_map(|host| host.storage_devices.iter().flatten()).filter(|storage| storage.total_space_bytes.is_some()).count()}),
        ),
        Err(error) => failure("hosts", error),
    }
    match &account {
        Ok(account) => emit(
            "account",
            json!({"outcome":"success", "profileAvailable":account.profile.is_some(), "friendCount":account.friends.as_ref().map(Vec::len), "friendsWithoutName":account.friends.as_ref().map(|friends| friends.iter().filter(|person| person.gamertag.is_empty() && person.display_name.is_empty()).count()), "notices":account.notices}),
        ),
        Err(error) => failure("account", error),
    }
    let history = match history {
        Ok(history) => history,
        Err(error) => {
            failure("history", &error);
            std::process::exit(1);
        }
    };
    emit(
        "history",
        json!({"outcome":"success", "count":history.items.len(), "summaryCount":history.items.iter().filter(|game| game.progress.is_some()).count(), "gamerscoreSum":history.items.iter().filter_map(|game| game.progress.as_ref()?.earned_gamerscore).map(u64::from).sum::<u64>(), "httpsArtworkCount":history.items.iter().filter(|game| game.artwork_url.as_deref().is_some_and(|url| url.starts_with("https://"))).count(), "hasContinuation":history.continuation.is_some(), "elapsedMs":started.elapsed().as_millis()}),
    );
    for (index, game) in history.items.iter().take(3).enumerate() {
        match PersonalService::achievements(&ctx, &game.xbox_title_id, None, "zh-CN").await {
            Ok(page) => emit(
                &format!("achievements-{index}"),
                json!({"outcome":"success", "count":page.items.len(), "hasContinuation":page.continuation.is_some()}),
            ),
            Err(error) => failure(&format!("achievements-{index}"), &error),
        }
    }
    let ids: Vec<_> = history
        .items
        .iter()
        .map(|game| game.xbox_title_id.clone())
        .collect();
    let started = Instant::now();
    for (index, ids) in ids.chunks(100).enumerate() {
        let batch_started = Instant::now();
        match PersonalService::playtimes(&ctx, ids.to_vec()).await {
            Ok(rows) => emit(
                &format!("playtimes-{index}"),
                json!({"outcome":"success", "requested":ids.len(), "returned":rows.len(), "available":rows.iter().filter(|row| row.minutes.is_some()).count(), "missing":rows.iter().filter(|row| row.minutes.is_none() && row.error.is_none()).count(), "failed":rows.iter().filter(|row| row.error.is_some()).count(), "elapsedMs":batch_started.elapsed().as_millis()}),
            ),
            Err(error) => failure(&format!("playtimes-{index}"), &error),
        }
    }
    emit(
        "playtimes-total",
        json!({"requested":ids.len(), "elapsedMs":started.elapsed().as_millis()}),
    );
    if let Some(game) = history.items.first() {
        match PersonalService::game(&ctx, &game.xbox_title_id).await {
            Ok(game) => emit(
                "gameDetail",
                json!({"outcome":"success", "available":game.is_some()}),
            ),
            Err(error) => failure("gameDetail", &error),
        }
    }
    if let Some(id) = hosts
        .as_ref()
        .ok()
        .and_then(|hosts| hosts.first())
        .and_then(|host| host.id.as_deref().or(host.device_id.as_deref()))
    {
        let scope = CloudCatalogScope {
            market: "US".into(),
            language: "zh-TW".into(),
        };
        match PersonalService::console(&ctx, id, &scope).await {
            Ok(library) => emit(
                "consoleLibrary",
                json!({"outcome":"success", "gameCount":library.games.as_ref().map(Vec::len), "storageCount":library.storage.as_ref().map(Vec::len), "powerStateAvailable":library.power_state.is_some(), "artworkCount":library.games.as_ref().map(|games| games.iter().filter(|game| game.artwork_url.is_some()).count()), "notices":library.notices}),
            ),
            Err(error) => failure("consoleLibrary", &error),
        }
    }
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .https_only(true)
        .build()
        .expect("HTTP initialization");
    for kind in [MediaKind::Screenshot, MediaKind::Clip] {
        match PersonalService::media(&ctx, kind, None).await {
            Ok(page) => {
                emit(
                    &format!("media-{kind:?}"),
                    json!({"outcome":"success", "count":page.items.len(), "hasContinuation":page.continuation.is_some(), "httpsResourceCount":page.items.iter().filter(|media| media.url.as_deref().is_some_and(|url| url.starts_with("https://"))).count()}),
                );
                if let Some(url) = page.items.first().and_then(|media| media.url.as_deref()) {
                    match client.get(url).header("Range", "bytes=0-63").send().await {
                        Ok(response) => emit(
                            &format!("mediaResource-{kind:?}"),
                            json!({"status":response.status().as_u16(), "contentType":response.headers().get("content-type").and_then(|value| value.to_str().ok())}),
                        ),
                        Err(error) => {
                            failure("mediaResource", &WebApiError::from_reqwest_safe(error))
                        }
                    }
                }
            }
            Err(error) => failure("media", &error),
        }
    }
    if let Some(url) = history
        .items
        .iter()
        .filter_map(|game| game.artwork_url.as_deref())
        .next()
    {
        match client.get(url).header("Range", "bytes=0-63").send().await {
            Ok(response) => emit(
                "artwork",
                json!({"status":response.status().as_u16(), "contentType":response.headers().get("content-type").and_then(|value| value.to_str().ok())}),
            ),
            Err(error) => failure("artwork", &WebApiError::from_reqwest_safe(error)),
        }
    }
    // 云准入独立交换 GSSV XSTS；保持当前应用、持久化凭据及串流会话状态。
    let core = &store["auth.tokens.core"];
    if let (Some(user_token), Some(private_jwk)) = (
        core["sisuToken"]["UserToken"]["Token"].as_str(),
        core["jwtKeys"].get("privateJwk"),
    ) {
        let auth = AuthApi::new();
        match auth
            .xsts_authorize(user_token, "http://gssv.xboxlive.com/", private_jwk)
            .await
        {
            Ok(gssv) => {
                for offering in ["xgpuweb", "xgpuwebf2p"] {
                    match auth.get_streaming_token(&gssv.Token, offering, "").await {
                        Ok(_) => emit(offering, json!({"outcome":"success"})),
                        Err(error) => failure(offering, &error),
                    }
                }
            }
            Err(error) => failure("cloudXsts", &error),
        }
    }
    emit("trace", json!({"path":recorder.path()}));
}
