use crate::mods::data::cache_repository::{
    CachedXcloudCatalogBaseEntry, CachedXcloudCatalogOverlayEntry, DataCacheRepository,
};
use crate::mods::data::session_resolver::resolve_web_token_claims;
use crate::mods::data::types::{
    DataSessionContext, DataXcloudCatalogPayload, XcloudCatalogCacheScope,
};
use xbox_cloud_catalog_flow::{CloudCatalogBaseEntry, CloudCatalogOverlayEntry, CloudCatalogScope};
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
        let region_host = self
            .resolve_xcloud_region(session)
            .map(|r| r.host)
            .unwrap_or_else(|| "public".into());
        let account_id = claims
            .xid
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(&claims.uhs);
        Some(XcloudCatalogCacheScope {
            account_id: account_id.to_string(),
            region_host,
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
        force_refresh: bool,
        progress: impl Fn(DataXcloudCatalogPayload) + Send + Sync,
    ) -> Result<XcloudCatalogRefreshOutcome, WebApiError> {
        let catalog_scope = CloudCatalogScope {
            market: scope.market.clone(),
            language: scope.language.clone(),
        };
        let public = scope.region_host == "public";
        let api = xbox_webapi::GamePassApi::new();
        let region = self.resolve_xcloud_region(session);
        let cloud = region.map(|r| xbox_webapi::XcloudApi::new(r.host, r.bearer_token));
        let previous = cache_repository
            .load_xcloud_catalog(scope)
            .map_err(|_| WebApiError::parse("目录缓存读取失败"))?;
        let mut entries = if public {
            let raw = api
                .get_sigl(
                    "ce573635-7c18-4d0c-9d68-90b932393470",
                    &scope.market,
                    &scope.language,
                )
                .await?;
            if !raw.is_array() {
                return Err(WebApiError::parse("invalid public catalog response"));
            }
            xbox_cloud_catalog_flow::extract_newest_product_ids(&raw)
                .into_iter()
                .map(|id| CachedXcloudCatalogOverlayEntry {
                    fallback_name: id.clone(),
                    product_id: id,
                    ..Default::default()
                })
                .collect::<Vec<_>>()
        } else {
            let raw = cloud
                .as_ref()
                .ok_or_else(|| WebApiError::parse("云游戏访问凭据暂缺"))?
                .get_titles()
                .await?;
            xbox_cloud_catalog_flow::build_overlay_load(
                &raw,
                &serde_json::json!({"results":[]}),
                &serde_json::json!([]),
                false,
                false,
            )?
            .entries
            .into_iter()
            .map(Self::to_cached_overlay_entry)
            .collect()
        };
        let status_key = format!("{}.status", DataCacheRepository::scoped_cache_key(scope));
        let mut sources = std::collections::HashMap::<String, String>::new();
        sources.insert("catalog".into(), "fresh".into());
        sources.insert("products".into(), "loading".into());
        let access = if public { "browse" } else { "account" }.to_string();
        let save = |entries: Vec<CachedXcloudCatalogOverlayEntry>,
                    sources: &std::collections::HashMap<String, String>,
                    refreshing: bool|
         -> Result<DataXcloudCatalogPayload, WebApiError> {
            cache_repository
                .save_xcloud_catalog_overlay(scope, entries)
                .map_err(|_| WebApiError::parse("目录缓存写入失败"))?;
            cache_repository
                .write_cloud_value(&status_key, sources)
                .map_err(|_| WebApiError::parse("目录缓存写入失败"))?;
            let loaded = cache_repository
                .load_xcloud_catalog(scope)
                .map_err(|_| WebApiError::parse("目录缓存读取失败"))?;
            Ok(DataXcloudCatalogPayload {
                scope_key: Some(DataCacheRepository::scoped_cache_key(scope)),
                titles: loaded.titles,
                cache_state: loaded.cache_state,
                updated_at: loaded.updated_at,
                refreshing,
                sources: sources.clone(),
                access: access.clone(),
                error: None,
            })
        };
        progress(save(entries.clone(), &sources, true)?);
        let cached = cache_repository
            .get_xcloud_catalog_base_snapshot(scope)
            .map_err(|_| WebApiError::parse("目录缓存读取失败"))?
            .unwrap_or_default();
        let now = DataCacheRepository::now_ms();
        let ids = entries
            .iter()
            .filter(|e| {
                force_refresh
                    || cached.entries.get(&e.product_id).is_none_or(|b| {
                        now.saturating_sub(if b.fetched_at == 0 {
                            cached.updated_at
                        } else {
                            b.fetched_at
                        }) > 24 * 60 * 60 * 1000
                    })
            })
            .map(|e| e.product_id.clone())
            .collect::<Vec<_>>();
        // 先补首批轻量资料，再并行读取栏目和剩余商品。
        let first = xbox_cloud_catalog_flow::hydrate_catalog_products(
            &api,
            &catalog_scope,
            &ids[..ids.len().min(75)],
        )
        .await;
        cache_repository
            .save_xcloud_catalog_base(
                scope,
                first
                    .entries
                    .into_iter()
                    .map(|(id, b)| (id, Self::to_cached_base_entry(b)))
                    .collect(),
            )
            .map_err(|_| WebApiError::parse("商品缓存写入失败"))?;
        progress(save(entries.clone(), &sources, true)?);
        let recent_query = async {
            match &cloud {
                Some(cloud) => cloud.get_recent_titles(25).await,
                None => Ok(serde_json::json!({"results":[]})),
            }
        };
        let (recent, newest, popular, leaving, rest) = tokio::join!(
            recent_query,
            api.get_sigl(
                "44a55037-770f-4bbf-bde5-a9fa27dba1da",
                &scope.market,
                &scope.language
            ),
            api.get_sigl(
                "e7590b22-e299-44db-ae22-25c61405454c",
                &scope.market,
                &scope.language
            ),
            api.get_sigl(
                "393f05bf-e596-4ef6-9487-6d4fa0eab987",
                &scope.market,
                &scope.language
            ),
            xbox_cloud_catalog_flow::hydrate_catalog_products(
                &api,
                &catalog_scope,
                &ids[ids.len().min(75)..]
            )
        );
        let recent = recent.and_then(|v| {
            v.get("results")
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|v| v.get("details")?.get("productId")?.as_str())
                        .map(str::to_uppercase)
                        .collect::<Vec<_>>()
                })
                .ok_or_else(|| WebApiError::parse("invalid recent titles"))
        });
        for (key, result) in [
            ("newest", newest),
            ("popular", popular),
            ("leaving", leaving),
        ] {
            let result = result.and_then(|v| {
                if v.is_array() {
                    Ok(xbox_cloud_catalog_flow::extract_newest_product_ids(&v))
                } else {
                    Err(WebApiError::parse("invalid discovery collection"))
                }
            });
            sources.insert(
                key.into(),
                if result.is_ok() {
                    "fresh"
                } else {
                    "unavailable"
                }
                .into(),
            );
            for entry in &mut entries {
                let previous = previous
                    .titles
                    .iter()
                    .find(|t| t.product_id == entry.product_id);
                let rank = match &result {
                    Ok(ids) => ids.iter().position(|id| id == &entry.product_id),
                    Err(_) => previous.and_then(|t| match key {
                        "newest" => t.new_rank,
                        "popular" => t.popular_rank,
                        _ => t.leaving_rank,
                    }),
                };
                match key {
                    "newest" => {
                        entry.new_rank = rank;
                        entry.is_new = rank.is_some();
                    }
                    "popular" => entry.popular_rank = rank,
                    _ => entry.leaving_rank = rank,
                }
            }
        }
        sources.insert(
            "recent".into(),
            if recent.is_ok() {
                "fresh"
            } else {
                "unavailable"
            }
            .into(),
        );
        for entry in &mut entries {
            entry.recent_rank = match &recent {
                Ok(ids) => ids.iter().position(|id| id == &entry.product_id),
                Err(_) => previous
                    .titles
                    .iter()
                    .find(|t| t.product_id == entry.product_id)
                    .and_then(|t| t.recent_rank),
            };
            entry.is_recently_played = entry.recent_rank.is_some();
        }
        sources.insert(
            "products".into(),
            if first.failed_chunk_count + rest.failed_chunk_count == 0 {
                "fresh"
            } else {
                "unavailable"
            }
            .into(),
        );
        cache_repository
            .save_xcloud_catalog_base(
                scope,
                rest.entries
                    .into_iter()
                    .map(|(id, b)| (id, Self::to_cached_base_entry(b)))
                    .collect(),
            )
            .map_err(|_| WebApiError::parse("商品缓存写入失败"))?;
        Ok(XcloudCatalogRefreshOutcome {
            payload: save(entries, &sources, false)?,
            missing_product_count: ids.len(),
        })
    }

    fn to_cached_base_entry(entry: CloudCatalogBaseEntry) -> CachedXcloudCatalogBaseEntry {
        CachedXcloudCatalogBaseEntry {
            fetched_at: 0,

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
            entitlement_known: entry.entitlement_known,
            recent_rank: entry.recent_rank,
            new_rank: entry.new_rank,
            popular_rank: None,
            leaving_rank: None,

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

    pub(crate) fn resolve_xcloud_region(
        &self,
        session: &DataSessionContext,
    ) -> Option<ResolvedXcloudRegion> {
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
pub(crate) struct ResolvedXcloudRegion {
    pub host: String,
    pub bearer_token: String,
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
            entitlement_known: false,
            recent_rank: None,
            new_rank: None,

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
