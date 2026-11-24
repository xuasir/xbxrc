use crate::{
    deserialize, query_failure::query_error, resolve_web_token_claims, XboxBridgeError,
    XboxCloudCatalogMetadata, XboxConsoleStorage, XboxInstalledGame, XboxInstalledLibrary,
    XboxQueryFailure, XboxQueryFailureKind,
};
use serde_json::Value;
use xbox_cloud_catalog_flow::{hydrate_catalog_products_with_budget, CloudCatalogScope};
use xbox_webapi::{
    library_root, library_string, parse_console_storage, parse_console_storage_for_device,
    parse_installed_games, GamePassApi, SmartglassApi,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum XboxInstalledSource {
    Games,
    Storage,
    Status,
    Artwork,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxInstalledSection {
    pub snapshot: XboxInstalledLibrary,
    pub artwork: Vec<XboxCloudCatalogMetadata>,
    pub failure: Option<XboxQueryFailure>,
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_installed_section(
    web_token_json: String,
    console_id: String,
    market: String,
    language: String,
    source: XboxInstalledSource,
    product_ids: Vec<String>,
) -> Result<XboxInstalledSection, XboxBridgeError> {
    let id = console_id.trim();
    if id.is_empty()
        || id.len() > 256
        || id.chars().any(char::is_control)
        || product_ids.len() > 500
    {
        return Err(XboxBridgeError::InvalidData(
            "invalid installed section identity".into(),
        ));
    }
    let claims = resolve_web_token_claims(&deserialize::<Value>(&web_token_json)?)?;
    let api = SmartglassApi::new(claims.uhs, claims.token);
    let mut result = XboxInstalledSection {
        snapshot: XboxInstalledLibrary {
            console_id: id.into(),
            games: None,
            storage: None,
            power_state: None,
            notices: Vec::new(),
        },
        artwork: Vec::new(),
        failure: None,
    };
    match source {
        XboxInstalledSource::Games => {
            let games = api
                .get_installed_apps(id)
                .await
                .and_then(|value| parse_installed_games(&value))
                .map_err(query_error)?;
            result.snapshot.games = Some(
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
                    .collect(),
            );
        }
        XboxInstalledSource::Storage => {
            let storage = match api
                .get_storage_devices(id)
                .await
                .and_then(|value| parse_console_storage(&value))
            {
                Ok(storage) => storage,
                Err(error) if error.to_status_code() == Some(401) => {
                    return Err(query_error(error))
                }
                Err(_) => api
                    .get_consoles_list()
                    .await
                    .and_then(|value| parse_console_storage_for_device(&value, id))
                    .map_err(query_error)?,
            };
            result.snapshot.storage = Some(
                storage
                    .into_iter()
                    .map(|device| XboxConsoleStorage {
                        id: device.id,
                        name: device.name,
                        free_bytes: device.free_bytes,
                        total_bytes: device.total_bytes,
                    })
                    .collect(),
            );
        }
        XboxInstalledSource::Status => {
            let status = api.get_console_status(id).await.map_err(query_error)?;
            result.snapshot.power_state = library_string(library_root(&status).get("powerState"));
        }
        XboxInstalledSource::Artwork => {
            if !product_ids.is_empty() {
                let hydration = hydrate_catalog_products_with_budget(
                    &GamePassApi::new(),
                    &CloudCatalogScope { market, language },
                    &product_ids,
                    std::time::Duration::from_secs(8),
                )
                .await;
                result.artwork = hydration
                    .entries
                    .into_values()
                    .map(|entry| XboxCloudCatalogMetadata {
                        product_id: entry.product_id,
                        name: entry.name,
                        publisher_name: entry.publisher_name,
                        description: entry.description,
                        tile_image_url: Some(entry.tile_image_url).filter(|v| !v.is_empty()),
                        poster_image_url: Some(entry.poster_image_url).filter(|v| !v.is_empty()),
                        hero_image_url: Some(entry.hero_image_url).filter(|v| !v.is_empty()),
                        categories: entry.categories,
                    })
                    .collect();
                if hydration.failed_chunk_count > 0 {
                    result.failure = Some(XboxQueryFailure {
                        kind: XboxQueryFailureKind::ServiceUnavailable,
                        status_code: None,
                        retriable: true,
                        retry_after_seconds: None,
                        network_error_kind: None,
                        service_code: None,
                        protocol_error_code: None,
                    });
                }
            }
        }
    }
    Ok(result)
}
