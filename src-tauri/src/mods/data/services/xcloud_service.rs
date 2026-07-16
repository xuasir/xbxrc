use crate::mods::data::cache_repository::{
    CachedXcloudCatalogBaseEntry, CachedXcloudCatalogOverlayEntry, DataCacheRepository,
};
use crate::mods::data::session_resolver::resolve_web_token_claims;
use crate::mods::data::types::{
    DataSessionContext, DataXcloudCatalogCacheState, DataXcloudCatalogPayload,
    XcloudCatalogCacheScope,
};
use xbox_cloud_catalog_flow::{
    CloudCatalogBaseEntry, CloudCatalogOverlayEntry, CloudCatalogScope, XboxCloudCatalogFlow,
};

const XCLOUD_CATALOG_DEFAULT_MARKET: &str = "US";

pub(crate) struct XcloudCatalogRefreshOutcome {
    pub payload: DataXcloudCatalogPayload,
    pub missing_product_count: usize,
}

pub struct XcloudService;

impl XcloudService {
    pub fn new() -> Self {
        Self
    }

    pub fn resolve_cache_scope(
        &self,
        session: &DataSessionContext,
    ) -> Option<XcloudCatalogCacheScope> {
        let claims = resolve_web_token_claims(&session.web_token)?;
        let region = Self::resolve_xcloud_region(session)?;
        let account_id = claims
            .xid
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(&claims.uhs);
        Some(XcloudCatalogCacheScope {
            account_id: account_id.to_string(),
            region_host: region.host,
            language: Self::resolve_catalog_language(),
            market: XCLOUD_CATALOG_DEFAULT_MARKET.to_string(),
        })
    }

    pub(crate) async fn refresh_catalog(
        &self,
        session: &DataSessionContext,
        scope: &XcloudCatalogCacheScope,
        cache_repository: &DataCacheRepository,
        _force_refresh: bool,
    ) -> Result<XcloudCatalogRefreshOutcome, String> {
        let Some(region) = Self::resolve_xcloud_region(session) else {
            return Ok(XcloudCatalogRefreshOutcome {
                payload: DataXcloudCatalogPayload {
                    titles: Vec::new(),
                    cache_state: DataXcloudCatalogCacheState::Miss,
                    updated_at: None,
                    refreshing: false,
                },
                missing_product_count: 0,
            });
        };

        let catalog_scope = CloudCatalogScope {
            market: scope.market.clone(),
            language: scope.language.clone(),
        };
        let catalog_flow = XboxCloudCatalogFlow::new(region.host, region.bearer_token);
        let overlay_load = catalog_flow
            .load_overlay(&catalog_scope)
            .await
            .map_err(|error| error.to_string())?;

        if !overlay_load.recent_source_available {
            log::warn!("[Data][xcloud] recent titles source unavailable; continuing without MRU");
        }
        if !overlay_load.newest_source_available {
            log::warn!("[Data][xcloud] Game Pass newest source unavailable; continuing without new markers");
        }

        let cached_base_snapshot = cache_repository
            .get_xcloud_catalog_base_snapshot(scope)?
            .filter(|snapshot| !snapshot.entries.is_empty())
            .unwrap_or_default();
        let missing_product_ids = if cached_base_snapshot.entries.is_empty() {
            overlay_load.product_ids.clone()
        } else {
            overlay_load
                .product_ids
                .iter()
                .filter(|product_id| !cached_base_snapshot.entries.contains_key(*product_id))
                .cloned()
                .collect::<Vec<_>>()
        };

        let hydration_load = catalog_flow
            .hydrate_products(&catalog_scope, &missing_product_ids)
            .await;
        if hydration_load.failed_chunk_count > 0 {
            log::warn!(
                "[Data][xcloud] Game Pass products hydration failed chunks={}",
                hydration_load.failed_chunk_count
            );
        }

        let mut merged_base_entries = cached_base_snapshot.entries;
        merged_base_entries.extend(
            hydration_load
                .entries
                .into_iter()
                .map(|(product_id, entry)| (product_id, Self::to_cached_base_entry(entry))),
        );
        let overlay_entries = overlay_load
            .entries
            .into_iter()
            .map(Self::to_cached_overlay_entry)
            .collect();

        cache_repository.save_xcloud_catalog_base(scope, merged_base_entries)?;
        cache_repository.save_xcloud_catalog_overlay(scope, overlay_entries)?;

        let loaded_snapshot = cache_repository.load_xcloud_catalog(scope)?;
        Ok(XcloudCatalogRefreshOutcome {
            payload: DataXcloudCatalogPayload {
                titles: loaded_snapshot.titles,
                cache_state: loaded_snapshot.cache_state,
                updated_at: loaded_snapshot.updated_at,
                refreshing: false,
            },
            missing_product_count: loaded_snapshot.missing_product_ids.len(),
        })
    }

    fn to_cached_base_entry(entry: CloudCatalogBaseEntry) -> CachedXcloudCatalogBaseEntry {
        CachedXcloudCatalogBaseEntry {
            product_id: entry.product_id,
            name: entry.name,
            publisher_name: entry.publisher_name,
            description: entry.description,
            tile_image_url: entry.tile_image_url,
            poster_image_url: entry.poster_image_url,
            categories: entry.categories,
        }
    }

    fn to_cached_overlay_entry(entry: CloudCatalogOverlayEntry) -> CachedXcloudCatalogOverlayEntry {
        CachedXcloudCatalogOverlayEntry {
            product_id: entry.product_id,
            title_id: entry.stream_title_id,
            xbox_title_id: entry.xbox_title_id,
            fallback_name: entry.fallback_name,
            supported_input_types: entry.supported_input_types,
            has_entitlement: entry.has_entitlement,
            is_recently_played: entry.is_recently_played,
            is_new: entry.is_new,
        }
    }

    fn resolve_xcloud_region(session: &DataSessionContext) -> Option<ResolvedXcloudRegion> {
        let token = session
            .streaming_tokens
            .get("xCloudToken")
            .or_else(|| session.streaming_tokens.get("xcloudToken"))?;
        let data = token.get("data").unwrap_or(token);

        let bearer_token = data.get("gsToken").and_then(|value| value.as_str())?;
        let regions = data
            .get("offeringSettings")
            .and_then(|value| value.get("regions"))
            .and_then(|value| value.as_array())?;

        let region = regions
            .iter()
            .find(|item| item.get("isDefault").and_then(|value| value.as_bool()) == Some(true))
            .or_else(|| regions.first())?;

        let base_uri = region.get("baseUri").and_then(|value| value.as_str())?;
        let host = base_uri
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/');

        if host.is_empty() {
            return None;
        }

        Some(ResolvedXcloudRegion {
            host: host.to_string(),
            bearer_token: bearer_token.to_string(),
        })
    }

    fn resolve_catalog_language() -> String {
        match std::env::var("LANG") {
            Ok(lang) if lang.to_lowercase().starts_with("zh") => "zh-TW".to_string(),
            _ => "en-US".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
struct ResolvedXcloudRegion {
    host: String,
    bearer_token: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cache_scope_uses_stable_xid_when_available() {
        let session = DataSessionContext {
            provider: "xbox".to_string(),
            app_level: 2,
            streaming_tokens: json!({
                "xCloudToken": {
                    "data": {
                        "gsToken": "token",
                        "offeringSettings": {
                            "regions": [{
                                "isDefault": true,
                                "baseUri": "https://wus.core.gssv-play-prod.xboxlive.com"
                            }]
                        }
                    }
                }
            }),
            web_token: json!({
                "data": {
                    "Token": "web-token",
                    "DisplayClaims": {
                        "xui": [{
                            "uhs": "volatile-uhs",
                            "xid": "stable-xid"
                        }]
                    }
                }
            }),
        };

        let scope = XcloudService::new()
            .resolve_cache_scope(&session)
            .expect("xcloud scope should resolve");

        assert_eq!(scope.account_id, "stable-xid");
        assert_eq!(scope.region_host, "wus.core.gssv-play-prod.xboxlive.com");
        assert_eq!(scope.market, "US");
    }

    #[test]
    fn maps_shared_entries_to_existing_cache_contract() {
        let base = XcloudService::to_cached_base_entry(CloudCatalogBaseEntry {
            product_id: "9ABC".to_string(),
            name: "Alpha".to_string(),
            publisher_name: "Publisher".to_string(),
            description: "Description".to_string(),
            tile_image_url: "tile".to_string(),
            poster_image_url: "poster".to_string(),
            hero_image_url: "hero".to_string(),
            categories: vec!["Action".to_string()],
        });
        let overlay = XcloudService::to_cached_overlay_entry(CloudCatalogOverlayEntry {
            product_id: "9ABC".to_string(),
            stream_title_id: "stream-alpha".to_string(),
            xbox_title_id: Some(123),
            fallback_name: "Alpha".to_string(),
            supported_input_types: vec!["Controller".to_string()],
            has_entitlement: true,
            is_recently_played: true,
            is_new: false,
        });

        assert_eq!(base.product_id, "9ABC");
        assert_eq!(base.poster_image_url, "poster");
        assert_eq!(overlay.title_id, "stream-alpha");
        assert_eq!(overlay.xbox_title_id, Some(123));
        assert!(overlay.is_recently_played);
    }
}
