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
use xbox_webapi::WebApiError;

const XCLOUD_CATALOG_DEFAULT_MARKET: &str = "US";

pub(crate) struct XcloudCatalogRefreshOutcome {
    pub payload: DataXcloudCatalogPayload,
    pub missing_product_count: usize,
}

pub struct XcloudService {
    config: Option<crate::mods::config::ConfigProviderRef>,
}

impl XcloudService {
    pub fn new() -> Self {
        Self { config: None }
    }

    pub fn with_config(config: crate::mods::config::ConfigProviderRef) -> Self {
        Self {
            config: Some(config),
        }
    }

    pub fn resolve_cache_scope(
        &self,
        session: &DataSessionContext,
        catalog_scope: &CloudCatalogScope,
    ) -> Option<XcloudCatalogCacheScope> {
        let claims = resolve_web_token_claims(&session.web_token)?;
        let region = self.resolve_xcloud_region(session)?;
        let account_id = claims
            .xid
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(&claims.uhs);
        Some(XcloudCatalogCacheScope {
            account_id: account_id.to_string(),
            region_host: region.host,
            language: catalog_scope.language.clone(),
            market: catalog_scope.market.clone(),
        })
    }

    pub(crate) fn unavailable_error(session: &DataSessionContext) -> Option<WebApiError> {
        crate::mods::auth::token_policy::cloud_token_unavailable_error(&session.streaming_tokens)
    }

    pub(crate) fn catalog_error(error: &WebApiError) -> String {
        if error.to_status_code() == Some(403)
            && matches!(
                error.safe_service_code().as_deref(),
                None | Some("Forbidden" | "AuthorizationFailed" | "NotAuthorized")
            )
        {
            return "云游戏服务拒绝访问，请检查服务支持的地区和账户资格".into();
        }
        super::personal_service::user_error(error)
    }

    pub(crate) async fn refresh_catalog(
        &self,
        session: &DataSessionContext,
        scope: &XcloudCatalogCacheScope,
        cache_repository: &DataCacheRepository,
        _force_refresh: bool,
    ) -> Result<XcloudCatalogRefreshOutcome, WebApiError> {
        let Some(region) = self.resolve_xcloud_region(session) else {
            return Ok(XcloudCatalogRefreshOutcome {
                payload: DataXcloudCatalogPayload {
                    scope_key: Some(DataCacheRepository::scoped_cache_key(scope)),
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
            .map_err(|error| match error {
                xbox_cloud_catalog_flow::CloudCatalogFlowError::WebApi(error) => error,
            })?;

        for failure in &overlay_load.source_failures {
            log::warn!(
                "[Data][xcloud] source={} error={}",
                failure.source,
                failure.error
            );
        }
        if !overlay_load.recent_source_available {
            log::warn!("[Data][xcloud] recent titles source unavailable; continuing without MRU");
        }
        if !overlay_load.newest_source_available {
            log::warn!("[Data][xcloud] Game Pass newest source unavailable; continuing without new markers");
        }

        let cached_base_snapshot = cache_repository
            .get_xcloud_catalog_base_snapshot(scope)
            .map_err(|_| WebApiError::parse("目录缓存读取失败，请重试"))?
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
        for failure in &hydration_load.source_failures {
            log::warn!(
                "[Data][xcloud] source={} error={}",
                failure.source,
                failure.error
            );
        }
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

        cache_repository
            .save_xcloud_catalog_base(scope, merged_base_entries)
            .map_err(|_| WebApiError::parse("目录缓存写入失败，请检查存储空间"))?;
        cache_repository
            .save_xcloud_catalog_overlay(scope, overlay_entries)
            .map_err(|_| WebApiError::parse("目录缓存写入失败，请检查存储空间"))?;

        let loaded_snapshot = cache_repository
            .load_xcloud_catalog(scope)
            .map_err(|_| WebApiError::parse("目录缓存读取失败，请重试"))?;
        Ok(XcloudCatalogRefreshOutcome {
            payload: DataXcloudCatalogPayload {
                scope_key: Some(DataCacheRepository::scoped_cache_key(scope)),
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

    fn resolve_xcloud_region(&self, session: &DataSessionContext) -> Option<ResolvedXcloudRegion> {
        let token = session
            .streaming_tokens
            .get("xCloudToken")
            .or_else(|| session.streaming_tokens.get("xcloudToken"))?;
        let access = xbox_streaming::parse_session_access_context(token).ok()?;
        let mut config = xbox_streaming::Config::default();
        if let Some(provider) = &self.config {
            let name = provider.resolve_cloud_region_preference(&access.regions);
            config.session.preferred_region_name = (!name.is_empty()).then_some(name);
        }
        let region =
            xbox_streaming::policy::session::compiler::pick_region(&access.regions, &config)
                .ok()?;
        Some(ResolvedXcloudRegion {
            host: region
                .base_uri
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .to_owned(),
            bearer_token: access.gs_token?,
        })
    }

    pub(crate) fn resolve_catalog_scope(locale: &str) -> CloudCatalogScope {
        CloudCatalogScope {
            market: XCLOUD_CATALOG_DEFAULT_MARKET.to_string(),
            language: if locale.trim().to_ascii_lowercase().starts_with("zh") {
                "zh-TW"
            } else {
                "en-US"
            }
            .to_string(),
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
    fn cloud_errors_preserve_network_failures_and_use_cloud_access_guidance() {
        let session = DataSessionContext {
            provider: "xbox".into(),
            app_level: 1,
            web_token: json!({}),
            streaming_tokens: json!({"_diagnostics": {"xCloudToken": {
                "xgpuweb": {"statusCode": 403, "serviceCode": "OfferingAccessDenied"},
                "xgpuwebf2p": {"networkErrorKind": "timeout", "message": "secret-body"}
            }}}),
        };
        let error = XcloudService::unavailable_error(&session).unwrap();
        assert_eq!(
            error.network_error_kind(),
            Some(xbox_webapi::NetworkErrorKind::Timeout)
        );
        assert!(XcloudService::catalog_error(&error).contains("超时"));
        let message = XcloudService::catalog_error(&WebApiError::http(403, "secret-body"));
        assert!(message.contains("地区和账户资格"));
        assert!(!message.contains("secret"));
        assert!(XcloudService::catalog_error(&WebApiError::http_with_code(
            403,
            "UnsupportedCountry",
            "secret-body"
        ))
        .contains("地区"));
    }

    #[test]
    fn catalog_scope_uses_application_locale() {
        for (locale, language) in [
            ("zh", "zh-TW"),
            ("zh-CN", "zh-TW"),
            (" ZH_tw ", "zh-TW"),
            ("en", "en-US"),
            ("en-GB", "en-US"),
            ("", "en-US"),
        ] {
            let scope = XcloudService::resolve_catalog_scope(locale);
            assert_eq!(scope.language, language);
            assert_eq!(scope.market, "US");
        }
    }

    #[test]
    fn cache_scope_uses_stable_xid_and_application_catalog_language() {
        let session = DataSessionContext {
            provider: "xbox".to_string(),
            app_level: 2,
            streaming_tokens: json!({
                "xCloudToken": {
                    "data": {
                        "gsToken": "token",
                        "offeringSettings": {
                            "regions": [{
                                "name": "WestUS",
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
            .resolve_cache_scope(&session, &XcloudService::resolve_catalog_scope("en"))
            .expect("xcloud scope should resolve");

        assert_eq!(scope.account_id, "stable-xid");
        assert_eq!(scope.region_host, "wus.core.gssv-play-prod.xboxlive.com");
        assert_eq!(scope.market, "US");
        assert_eq!(scope.language, "en-US");
        let chinese_scope = XcloudService::new()
            .resolve_cache_scope(&session, &XcloudService::resolve_catalog_scope("zh"))
            .unwrap();
        assert_eq!(chinese_scope.account_id, scope.account_id);
        assert_eq!(chinese_scope.language, "zh-TW");
        assert_ne!(
            DataCacheRepository::scoped_cache_key(&scope),
            DataCacheRepository::scoped_cache_key(&chinese_scope)
        );
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
