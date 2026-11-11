use super::*;
use crate::mods::data::types::{
    DataCloudDetail, DataCloudPreferences, DataCloudWait, DataXcloudTitleSummary,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::{Mutex, Notify, Semaphore};

pub(super) struct CloudQueryState {
    titles: Mutex<HashMap<String, Arc<Mutex<Option<(u64, DataXcloudTitleSummary)>>>>>,
    waits: Mutex<HashMap<String, Arc<Mutex<Option<DataCloudWait>>>>>,
    cancellations: Mutex<HashMap<String, Arc<Notify>>>,
    capacity: Semaphore,
}
impl CloudQueryState {
    pub fn new() -> Self {
        Self {
            capacity: Semaphore::new(2),
            titles: Mutex::new(HashMap::new()),
            waits: Mutex::new(HashMap::new()),
            cancellations: Mutex::new(HashMap::new()),
        }
    }
}
#[derive(Serialize, Deserialize)]
struct DetailCache {
    detail: xbox_cloud_catalog_flow::CloudProductDetail,
    updated_at: u64,
}

impl DataService {
    pub(super) fn cloud_scope_key(&self) -> Result<String, DataError> {
        let session = self
            .session_resolver
            .current_session()
            .map_err(DataError::from)?;
        let account = session
            .as_ref()
            .and_then(|s| {
                crate::mods::data::session_resolver::resolve_web_token_claims(&s.web_token)
            })
            .map(|c| c.account_scope().to_string())
            .unwrap_or_else(|| "anonymous".into());
        use sha2::{Digest, Sha256};
        Ok(format!(
            "data.cloudPreferences.v1.{:x}",
            Sha256::digest(account.as_bytes())
        ))
    }
    pub(super) async fn cloud_detail(
        &self,
        product_id: &str,
    ) -> Result<DataCloudDetail, DataError> {
        let id = normalized_product(product_id)?;
        let scope = self.catalog_scope()?;
        let key = format!(
            "data.cloudDetail.v1.{}.{}.{}",
            scope.market, scope.language, id
        );
        let cached = self
            .cache_repository
            .read_cloud_value::<DetailCache>(&key)?;
        let now = DataCacheRepository::now_ms();
        if let Some(cached) = cached
            .as_ref()
            .filter(|c| now.saturating_sub(c.updated_at) < 24 * 60 * 60 * 1000)
        {
            return Ok(DataCloudDetail {
                detail: cached.detail.clone(),
                updated_at: cached.updated_at,
                stale: false,
            });
        }
        let result = xbox_webapi::GamePassApi::new()
            .get_products_with_hydration(
                &[id.clone()],
                &scope.market,
                &scope.language,
                "RemoteHighSapphire0",
            )
            .await;
        match result {
            Ok(raw) => {
                let detail = xbox_cloud_catalog_flow::extract_product_detail(&raw, &id)
                    .ok_or("游戏资料暂缺，请重试")?;
                let fetched = DataCacheRepository::now_ms();
                self.cache_repository.write_cloud_value(
                    &key,
                    &DetailCache {
                        detail: detail.clone(),
                        updated_at: fetched,
                    },
                )?;
                Ok(DataCloudDetail {
                    detail,
                    updated_at: fetched,
                    stale: false,
                })
            }
            Err(error) => match cached
                .filter(|c| now.saturating_sub(c.updated_at) <= 7 * 24 * 60 * 60 * 1000)
            {
                Some(cached) => Ok(DataCloudDetail {
                    detail: cached.detail,
                    updated_at: cached.updated_at,
                    stale: true,
                }),
                None => Err(catalog_data_error(error)),
            },
        }
    }
    pub(super) fn cloud_preferences(&self) -> Result<(String, DataCloudPreferences), DataError> {
        let key = self.cloud_scope_key()?;
        Ok((
            key.clone(),
            self.cache_repository
                .read_cloud_value(&key)?
                .unwrap_or_default(),
        ))
    }
    pub(super) fn save_cloud_preferences(
        &self,
        key: &str,
        mut prefs: DataCloudPreferences,
    ) -> Result<DataCloudPreferences, DataError> {
        if key != self.cloud_scope_key()? {
            return Err("账户已更新，请重试".into());
        }
        if prefs.favorites.len() > 2000 || prefs.pinned.len() > 2000 {
            return Err("收藏数量超出上限".into());
        }
        let mut seen = std::collections::HashSet::new();
        prefs.favorites.retain(|title| {
            normalized_product(&title.product_id).is_ok() && seen.insert(title.product_id.clone())
        });
        prefs.pinned.retain(|id| seen.contains(id));
        if !matches!(
            prefs.section.as_str(),
            "" | "xcloud-recent"
                | "xcloud-new"
                | "xcloud-popular"
                | "xcloud-leaving"
                | "xcloud-favorites"
                | "xcloud-all"
        ) {
            prefs.section.clear();
        }
        self.cache_repository.write_cloud_value(key, &prefs)?;
        Ok(prefs)
    }
    pub(super) async fn cloud_title(
        &self,
        id: &str,
        id_type: &str,
    ) -> Result<DataXcloudTitleSummary, DataError> {
        let scope = self.catalog_scope()?;
        let identity = self.cloud_scope_key()?;
        let existing = self
            .session_resolver
            .current_session()
            .map_err(DataError::from)?;
        let region = existing
            .as_ref()
            .and_then(|session| self.xcloud_service.resolve_xcloud_region(session))
            .map(|r| r.host)
            .unwrap_or_default();
        let key = format!(
            "{identity}:{region}:{}:{}:{id_type}:{id}",
            scope.market, scope.language
        );
        let slot = {
            let mut titles = self.cloud_queries.titles.lock().await;
            if titles.len() >= 256 {
                titles.retain(|_, v| Arc::strong_count(v) > 1);
            }
            titles
                .entry(key)
                .or_insert_with(|| Arc::new(Mutex::new(None)))
                .clone()
        };
        let mut cached = slot.lock().await;
        if identity != self.cloud_scope_key()? {
            return Err("账户已更新".into());
        }
        if let Some((fetched, title)) = cached
            .as_ref()
            .filter(|(time, _)| DataCacheRepository::now_ms().saturating_sub(*time) < 60_000)
        {
            let _ = fetched;
            return Ok(title.clone());
        }
        let title = self
            .session_resolver
            .run_scoped_query(AuthTokenScope::Cloud, |session| {
                let scope = &scope;
                async move {
                    let r = self
                        .xcloud_service
                        .resolve_xcloud_region(&session)
                        .ok_or_else(|| {
                            XcloudService::unavailable_error(&session).unwrap_or_else(|| {
                                xbox_webapi::WebApiError::parse("请登录并检查云游戏地区与账户资格")
                            })
                        })?;
                    let raw = xbox_webapi::XcloudApi::new(r.host, r.bearer_token)
                        .get_titles_by_ids(&[id.to_string()], id_type)
                        .await?;
                    let loaded = xbox_cloud_catalog_flow::build_overlay_load(
                        &raw,
                        &serde_json::json!({"results":[]}),
                        &serde_json::json!([]),
                        false,
                        false,
                    )?;
                    let entry = loaded
                        .entries
                        .into_iter()
                        .find(|e| {
                            if id_type == "productId" {
                                e.product_id == id.to_uppercase()
                            } else {
                                e.stream_title_id == id
                            }
                        })
                        .ok_or_else(|| {
                            xbox_webapi::WebApiError::parse("此游戏当前暂未提供云游戏访问")
                        })?;
                    let cache_scope = self
                        .xcloud_service
                        .resolve_cache_scope(&session, scope)
                        .ok_or_else(|| xbox_webapi::WebApiError::parse("账户访问暂不可用"))?;
                    let cached = self
                        .cache_repository
                        .load_xcloud_catalog(&cache_scope)
                        .ok()
                        .and_then(|c| {
                            c.titles
                                .into_iter()
                                .find(|t| t.product_id == entry.product_id)
                        });
                    Ok(DataXcloudTitleSummary {
                        id: entry.product_id.clone(),
                        name: cached
                            .as_ref()
                            .map(|t| t.name.clone())
                            .unwrap_or(entry.fallback_name.clone()),
                        product_id: entry.product_id,
                        title_id: entry.stream_title_id,
                        xbox_title_id: entry.xbox_title_id,
                        aliases: vec![entry.fallback_name],
                        publisher_name: cached
                            .as_ref()
                            .map(|t| t.publisher_name.clone())
                            .unwrap_or_default(),
                        description: cached
                            .as_ref()
                            .map(|t| t.description.clone())
                            .unwrap_or_default(),
                        tile_image_url: cached
                            .as_ref()
                            .map(|t| t.tile_image_url.clone())
                            .unwrap_or_default(),
                        poster_image_url: cached
                            .as_ref()
                            .map(|t| t.poster_image_url.clone())
                            .unwrap_or_default(),
                        categories: cached
                            .as_ref()
                            .map(|t| t.categories.clone())
                            .unwrap_or_default(),
                        supported_input_types: entry.supported_input_types,
                        has_entitlement: entry.has_entitlement,
                        entitlement_known: entry.entitlement_known,
                        is_recently_played: false,
                        is_new: false,
                        recent_rank: None,
                        new_rank: None,
                        popular_rank: None,
                        leaving_rank: None,
                    })
                }
            })
            .await
            .map_err(catalog_data_error)?;
        if identity != self.cloud_scope_key()? || scope != self.catalog_scope()? {
            return Err("账户或目录设置已更新".into());
        }
        *cached = Some((DataCacheRepository::now_ms(), title.clone()));
        Ok(title)
    }
    pub(super) async fn cloud_input(
        &self,
        stream_id: &str,
    ) -> Result<DataStreamingTitleInputConfig, DataError> {
        let title = self.cloud_title(stream_id, "titleId").await?;
        let xbox_id = title.xbox_title_id.ok_or("Xbox 游戏标识暂缺")?.to_string();
        let config = self
            .session_resolver
            .run_scoped_query(AuthTokenScope::Cloud, |session| {
                let xbox_id = &xbox_id;
                async move {
                    let r = self
                        .xcloud_service
                        .resolve_xcloud_region(&session)
                        .ok_or_else(|| xbox_webapi::WebApiError::parse("云访问暂不可用"))?;
                    xbox_webapi::XcloudApi::new(r.host, r.bearer_token)
                        .input_configs(xbox_id)
                        .await
                }
            })
            .await
            .map_err(catalog_data_error)?;
        Ok(DataStreamingTitleInputConfig {
            xbox_title_id: xbox_id,
            config,
        })
    }
    pub(super) async fn cloud_wait(
        &self,
        product: &str,
        request_id: &str,
    ) -> Result<DataCloudWait, DataError> {
        let product = normalized_product(product)?;
        if request_id.len() > 128 {
            return Err("查询标识无效".into());
        }
        let identity = self.cloud_scope_key()?;
        let catalog = self.catalog_scope()?;
        let session = self
            .session_resolver
            .current_session()
            .map_err(DataError::from)?
            .ok_or("请登录后查询等待时间")?;
        let region = self
            .xcloud_service
            .resolve_xcloud_region(&session)
            .ok_or("云访问暂不可用")?;
        let key = format!(
            "{identity}:{}:{}:{}:{product}",
            region.host, catalog.market, catalog.language
        );
        let slot = {
            let mut waits = self.cloud_queries.waits.lock().await;
            if waits.len() >= 256 {
                waits.retain(|_, v| Arc::strong_count(v) > 1);
            }
            waits
                .entry(key)
                .or_insert_with(|| Arc::new(Mutex::new(None)))
                .clone()
        };
        let signal = Arc::new(Notify::new());
        {
            let mut cancellations = self.cloud_queries.cancellations.lock().await;
            if cancellations.len() >= 128 {
                return Err("等待查询繁忙，请稍后重试".into());
            }
            cancellations.insert(request_id.into(), signal.clone());
        }
        let work = async {
            let mut cached = slot.lock().await;
            if identity != self.cloud_scope_key()? || catalog != self.catalog_scope()? {
                return Err("账户或目录设置已更新".into());
            }
            if let Some(value) = cached
                .as_ref()
                .filter(|v| DataCacheRepository::now_ms().saturating_sub(v.updated_at) < 60_000)
            {
                return Ok(value.clone());
            }
            let _permit = self
                .cloud_queries
                .capacity
                .acquire()
                .await
                .map_err(|_| DataError::from("等待查询已关闭"))?;
            let title = self.cloud_title(&product, "productId").await?;
            let value = self
                .session_resolver
                .run_scoped_query(AuthTokenScope::Cloud, |session| {
                    let title = &title;
                    let region = &region;
                    async move {
                        let r = self
                            .xcloud_service
                            .resolve_xcloud_region(&session)
                            .ok_or_else(|| xbox_webapi::WebApiError::parse("云访问暂不可用"))?;
                        if r.host != region.host {
                            return Err(xbox_webapi::WebApiError::parse("云区域已更新"));
                        }
                        xbox_webapi::XcloudApi::new(r.host, r.bearer_token)
                            .get_waiting_time(&title.title_id)
                            .await
                    }
                })
                .await
                .map_err(catalog_data_error)?;
            if identity != self.cloud_scope_key()? || catalog != self.catalog_scope()? {
                return Err("账户或目录设置已更新".into());
            }
            let result = DataCloudWait {
                seconds: value
                    .get("estimatedAllocationTimeInSeconds")
                    .and_then(serde_json::Value::as_u64),
                updated_at: DataCacheRepository::now_ms(),
            };
            *cached = Some(result.clone());
            Ok(result)
        };
        let result = tokio::select! { result=work=>result, _=signal.notified()=>Ok(DataCloudWait { seconds: None, updated_at: 0 }) };
        self.cloud_queries
            .cancellations
            .lock()
            .await
            .remove(request_id);
        result
    }
    pub(super) async fn cancel_cloud_wait(&self, request_id: &str) {
        if let Some(signal) = self
            .cloud_queries
            .cancellations
            .lock()
            .await
            .remove(request_id)
        {
            signal.notify_one();
        }
    }
}
fn normalized_product(id: &str) -> Result<String, DataError> {
    let id = id.trim().to_ascii_uppercase();
    if id.is_empty() || id.len() > 64 || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err("商品标识无效".into());
    }
    Ok(id)
}
