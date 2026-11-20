use crate::{deserialize, resolve_web_token_claims, XboxBridgeError};
use serde_json::Value;
use xbox_cloud_catalog_flow::{hydrate_catalog_products_with_budget, CloudCatalogScope};
use xbox_webapi::{
    library_root, library_string, parse_console_storage, parse_console_storage_for_device,
    parse_installed_games, parse_people, GamePassApi, MediaApi, MediaKind, PeopleApi,
    SmartglassApi,
};

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxFriend {
    pub id: String,
    pub display_name: String,
    pub gamertag: String,
    pub avatar_url: Option<String>,
    pub gamerscore: Option<String>,
    pub presence_state: Option<String>,
    pub presence_text: Option<String>,
    pub current_title_id: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxMediaItem {
    pub id: String,
    pub kind: String,
    pub title_id: Option<String>,
    pub title_name: String,
    pub captured_at: Option<String>,
    pub duration_seconds: Option<u64>,
    pub thumbnail_url: Option<String>,
    pub url: Option<String>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxMediaPage {
    pub items: Vec<XboxMediaItem>,
    pub continuation: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxInstalledGame {
    pub id: String,
    pub title_id: Option<String>,
    pub product_id: Option<String>,
    pub name: String,
    pub artwork_url: Option<String>,
    pub storage_device_id: Option<String>,
    pub size_bytes: Option<u64>,
    pub installed_at: Option<String>,
    pub last_played_at: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxConsoleStorage {
    pub id: String,
    pub name: String,
    pub free_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxInstalledLibrary {
    pub console_id: String,
    pub games: Option<Vec<XboxInstalledGame>>,
    pub storage: Option<Vec<XboxConsoleStorage>>,
    pub power_state: Option<String>,
    pub notices: Vec<String>,
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_friends(web_token_json: String) -> Result<Vec<XboxFriend>, XboxBridgeError> {
    let claims = resolve_web_token_claims(&deserialize::<Value>(&web_token_json)?)?;
    let value = PeopleApi::new(claims.uhs, claims.token)
        .get_friends()
        .await
        .map_err(data_error)?;
    Ok(parse_people(&value)
        .map_err(data_error)?
        .into_iter()
        .map(|person| XboxFriend {
            id: person.xuid,
            display_name: person.display_name,
            gamertag: person.gamertag,
            avatar_url: person.avatar_url,
            gamerscore: person.gamerscore,
            presence_state: person.presence_state,
            presence_text: person.presence_text,
            current_title_id: person.current_title_id,
        })
        .collect())
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_media_page(
    web_token_json: String,
    kind: String,
    continuation: Option<String>,
) -> Result<XboxMediaPage, XboxBridgeError> {
    let kind = match kind.as_str() {
        "screenshot" => MediaKind::Screenshot,
        "clip" => MediaKind::Clip,
        _ => return Err(XboxBridgeError::InvalidData("invalid media kind".into())),
    };
    if continuation
        .as_ref()
        .is_some_and(|token| token.len() > 4_096 || token.chars().any(char::is_control))
    {
        return Err(XboxBridgeError::InvalidData(
            "invalid media continuation".into(),
        ));
    }
    let claims = resolve_web_token_claims(&deserialize::<Value>(&web_token_json)?)?;
    let xuid = claims
        .xuid
        .ok_or_else(|| XboxBridgeError::InvalidData("missing account identity".into()))?;
    let page = MediaApi::new(claims.uhs, claims.token)
        .search(&xuid, kind, continuation.as_deref())
        .await
        .map_err(data_error)?;
    Ok(XboxMediaPage {
        items: page
            .items
            .into_iter()
            .map(|item| XboxMediaItem {
                id: item.id,
                kind: match item.kind {
                    MediaKind::Screenshot => "screenshot",
                    MediaKind::Clip => "clip",
                }
                .into(),
                title_id: item.xbox_title_id,
                title_name: item.title_name,
                captured_at: item.captured_at,
                duration_seconds: item.duration_seconds,
                thumbnail_url: item.thumbnail_url,
                url: item.url,
                expires_at: item.expires_at,
            })
            .collect(),
        continuation: page.continuation,
    })
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_installed_library(
    web_token_json: String,
    console_id: String,
    market: String,
    language: String,
) -> Result<XboxInstalledLibrary, XboxBridgeError> {
    let id = console_id.trim();
    if id.is_empty() || id.len() > 256 || id.chars().any(char::is_control) {
        return Err(XboxBridgeError::InvalidData(
            "invalid console identity".into(),
        ));
    }
    let claims = resolve_web_token_claims(&deserialize::<Value>(&web_token_json)?)?;
    let api = SmartglassApi::new(claims.uhs, claims.token);
    let (games, storage, status) = tokio::join!(
        api.get_installed_apps(id),
        api.get_storage_devices(id),
        api.get_console_status(id)
    );
    for error in [
        games.as_ref().err(),
        storage.as_ref().err(),
        status.as_ref().err(),
    ]
    .into_iter()
    .flatten()
    {
        if error.to_status_code() == Some(401) {
            return Err(data_error(error.clone()));
        }
    }
    let mut notices = Vec::new();
    let mut games = match games.and_then(|value| parse_installed_games(&value)) {
        Ok(games) => Some(games),
        Err(_) => {
            notices.push("安装游戏暂时无法载入，请刷新重试".into());
            None
        }
    };
    let storage = match storage.and_then(|value| parse_console_storage(&value)) {
        Ok(storage) => Some(storage),
        Err(_) => match api.get_consoles_list().await {
            Ok(value) => match parse_console_storage_for_device(&value, id) {
                Ok(storage) => Some(storage),
                Err(_) => {
                    notices.push("存储信息暂时无法载入".into());
                    None
                }
            },
            Err(error) if error.to_status_code() == Some(401) => return Err(data_error(error)),
            Err(_) => {
                notices.push("存储信息暂时无法载入".into());
                None
            }
        },
    };
    let power_state = status
        .ok()
        .and_then(|value| library_string(library_root(&value).get("powerState")));
    if power_state.is_none() {
        notices.push("主机状态暂时无法载入".into());
    }
    if let Some(games) = &mut games {
        let product_ids = games
            .iter()
            .filter_map(|game| game.product_id.clone())
            .collect::<Vec<_>>();
        if !product_ids.is_empty() {
            let scope = CloudCatalogScope {
                market: bounded_locale(&market, "US"),
                language: bounded_locale(&language, "en-US"),
            };
            let hydration = hydrate_catalog_products_with_budget(
                &GamePassApi::new(),
                &scope,
                &product_ids,
                std::time::Duration::from_secs(8),
            )
            .await;
            for game in games {
                if let Some(entry) = game
                    .product_id
                    .as_ref()
                    .and_then(|id| hydration.entries.get(id))
                {
                    game.artwork_url = [
                        entry.poster_image_url.as_str(),
                        entry.tile_image_url.as_str(),
                        entry.hero_image_url.as_str(),
                    ]
                    .into_iter()
                    .find(|url| !url.is_empty())
                    .map(str::to_string)
                    .or_else(|| game.artwork_url.clone());
                }
            }
            if hydration.failed_chunk_count > 0 {
                notices.push("部分封面同步中，安装信息已保留".into());
            }
        }
    }
    Ok(XboxInstalledLibrary {
        console_id: id.into(),
        games: games.map(|games| {
            games
                .into_iter()
                .map(|game| XboxInstalledGame {
                    id: game.id,
                    title_id: game.xbox_title_id,
                    product_id: game.product_id,
                    name: game.name,
                    artwork_url: game.artwork_url,
                    storage_device_id: game.storage_device_id,
                    size_bytes: game.size_bytes,
                    installed_at: game.installed_at,
                    last_played_at: game.last_played_at,
                })
                .collect()
        }),
        storage: storage.map(|devices| {
            devices
                .into_iter()
                .map(|device| XboxConsoleStorage {
                    id: device.id,
                    name: device.name,
                    free_bytes: device.free_bytes,
                    total_bytes: device.total_bytes,
                })
                .collect()
        }),
        power_state,
        notices,
    })
}

fn data_error(error: xbox_webapi::WebApiError) -> XboxBridgeError {
    crate::query_failure::query_error(error)
}
fn bounded_locale(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() || value.len() > 32 || value.chars().any(char::is_control) {
        fallback.into()
    } else {
        value.into()
    }
}
