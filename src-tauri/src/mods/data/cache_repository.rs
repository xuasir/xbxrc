use crate::mods::data::types::{
    DataUserProfile, DataXcloudCatalogCacheState, DataXcloudTitleSummary, XcloudCatalogCacheScope,
};
use crate::settings_store::SettingsStoreResolver;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::AppHandle;

const PROFILE_CACHE_KEY: &str = "data.profileCache";
pub(crate) const XCLOUD_CATALOG_CACHE_PREFIX: &str = "data.xcloudCatalog.v2";
pub(crate) const XCLOUD_CATALOG_BASE_RENDERABLE_TTL_MS: u64 = 7 * 24 * 60 * 60 * 1000;
pub(crate) const XCLOUD_CATALOG_OVERLAY_FRESH_TTL_MS: u64 = 10 * 60 * 1000;
pub(crate) const XCLOUD_CATALOG_OVERLAY_RENDERABLE_TTL_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CachedXcloudCatalogBaseEntry {
    #[serde(default)]
    pub fetched_at: u64,
    pub product_id: String,
    pub name: String,
    pub publisher_name: String,
    pub description: String,
    pub tile_image_url: String,
    pub poster_image_url: String,
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CachedXcloudCatalogBaseSnapshot {
    pub updated_at: u64,
    pub entries: HashMap<String, CachedXcloudCatalogBaseEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CachedXcloudCatalogOverlayEntry {
    pub product_id: String,
    pub title_id: String,
    pub xbox_title_id: Option<u64>,
    pub fallback_name: String,
    pub supported_input_types: Vec<String>,
    pub has_entitlement: bool,
    #[serde(default)]
    pub entitlement_known: bool,
    #[serde(default)]
    pub recent_rank: Option<usize>,
    #[serde(default)]
    pub new_rank: Option<usize>,
    #[serde(default)]
    pub popular_rank: Option<usize>,
    #[serde(default)]
    pub leaving_rank: Option<usize>,
    pub is_recently_played: bool,
    pub is_new: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CachedXcloudCatalogOverlaySnapshot {
    pub updated_at: u64,
    pub entries: Vec<CachedXcloudCatalogOverlayEntry>,
}

#[derive(Debug, Clone)]
pub(crate) struct LoadedXcloudCatalogSnapshot {
    pub titles: Vec<DataXcloudTitleSummary>,
    pub updated_at: Option<u64>,
    pub cache_state: DataXcloudCatalogCacheState,
    pub needs_refresh: bool,
    pub missing_product_ids: Vec<String>,
    pub hit_level: &'static str,
}

pub struct DataCacheRepository {
    settings_store: SettingsStoreResolver,
    memory_base_cache: Mutex<HashMap<String, CachedXcloudCatalogBaseSnapshot>>,
    memory_overlay_cache: Mutex<HashMap<String, CachedXcloudCatalogOverlaySnapshot>>,
}

fn scoped_profile_settings(value: &Value, account_scope: Option<&str>) -> HashMap<String, String> {
    let Some(scope) = account_scope.filter(|scope| !scope.is_empty()) else {
        return HashMap::new();
    };
    if value.get("accountScope").and_then(Value::as_str) != Some(scope) {
        return HashMap::new();
    }
    value
        .get("settings")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| value.as_str().map(|value| (key.clone(), value.to_string())))
        .collect()
}

impl DataCacheRepository {
    pub fn new(app_handle: AppHandle) -> Self {
        Self {
            settings_store: SettingsStoreResolver::new(app_handle),
            memory_base_cache: Mutex::new(HashMap::new()),
            memory_overlay_cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn save_cached_profile(
        &self,
        account_scope: &str,
        profile: &DataUserProfile,
    ) -> Result<(), String> {
        let mut profile_patch = Map::new();
        for (key, value) in &profile.settings {
            profile_patch.insert(key.clone(), Value::String(value.clone()));
        }

        let store = self.settings_store.open_write()?;
        store.store().set(
            PROFILE_CACHE_KEY,
            json!({ "accountScope": account_scope, "settings": profile_patch }),
        );
        store.save()
    }

    pub fn clear_cached_profile(&self) -> Result<(), String> {
        let store = self.settings_store.open_write()?;
        store.store().delete(PROFILE_CACHE_KEY);
        store.save()
    }

    pub fn get_cached_profile(
        &self,
        account_scope: Option<&str>,
        app_level: u32,
    ) -> Result<DataUserProfile, String> {
        let store = self.settings_store.open_read()?;

        let settings_value = store.store().get(PROFILE_CACHE_KEY).unwrap_or(Value::Null);
        let settings = scoped_profile_settings(&settings_value, account_scope);

        let gamertag = settings.get("Gamertag").cloned().unwrap_or_default();

        Ok(DataUserProfile {
            signed_in: !gamertag.is_empty(),
            game_display_name: settings.get("GameDisplayName").cloned().unwrap_or_default(),
            game_display_pic_raw: settings
                .get("GameDisplayPicRaw")
                .cloned()
                .unwrap_or_default(),
            gamertag,
            gamerscore: settings.get("Gamerscore").cloned().unwrap_or_default(),
            settings,
            app_level,
        })
    }

    pub(crate) fn load_xcloud_catalog(
        &self,
        scope: &XcloudCatalogCacheScope,
    ) -> Result<LoadedXcloudCatalogSnapshot, String> {
        let scoped_key = Self::scoped_cache_key(scope);
        let (base_snapshot, overlay_snapshot, hit_level) =
            if let Some(pair) = self.get_memory_catalog_pair(&scoped_key) {
                (pair.0, pair.1, "memory")
            } else {
                let pair = self.read_disk_catalog_pair(scope)?;
                self.cache_catalog_pair(&scoped_key, &pair.0, &pair.1);
                (pair.0, pair.1, "disk")
            };

        Ok(Self::build_loaded_snapshot(
            base_snapshot.as_ref(),
            overlay_snapshot.as_ref(),
            hit_level,
        ))
    }

    pub(crate) fn get_xcloud_catalog_base_snapshot(
        &self,
        scope: &XcloudCatalogCacheScope,
    ) -> Result<Option<CachedXcloudCatalogBaseSnapshot>, String> {
        let scoped_key = Self::scoped_cache_key(scope);
        if let Some(snapshot) = self.memory_base_cache_guard().get(&scoped_key).cloned() {
            return Ok(Some(snapshot));
        }

        let store = self.settings_store.open_read()?;
        let snapshot: Option<CachedXcloudCatalogBaseSnapshot> = store
            .store()
            .get(Self::base_store_key(scope))
            .and_then(|value| serde_json::from_value(value.clone()).ok());
        let snapshot = snapshot.or_else(|| Self::find_compatible_base_snapshot(&store, scope));
        if let Some(snapshot) = &snapshot {
            self.memory_base_cache_guard()
                .insert(scoped_key, snapshot.clone());
        }
        Ok(snapshot)
    }

    pub(crate) fn read_cloud_value<T: serde::de::DeserializeOwned>(
        &self,
        key: &str,
    ) -> Result<Option<T>, String> {
        let store = self.settings_store.open_read()?;
        Ok(store
            .store()
            .get(key)
            .and_then(|v| serde_json::from_value(v).ok()))
    }

    pub(crate) fn write_cloud_value<T: Serialize>(
        &self,
        key: &str,
        value: &T,
    ) -> Result<(), String> {
        let store = self.settings_store.open_write()?;
        store.store().set(
            key,
            serde_json::to_value(value).map_err(|_| "资料保存失败")?,
        );
        store.save()
    }

    pub(crate) fn save_xcloud_catalog_base(
        &self,
        scope: &XcloudCatalogCacheScope,
        entries: HashMap<String, CachedXcloudCatalogBaseEntry>,
    ) -> Result<(), String> {
        if entries.is_empty() {
            return Ok(());
        }
        let now = Self::now_ms();
        let mut merged = self
            .get_xcloud_catalog_base_snapshot(scope)?
            .unwrap_or_default();
        for entry in merged.entries.values_mut() {
            if entry.fetched_at == 0 {
                entry.fetched_at = merged.updated_at;
            }
        }
        for (id, mut entry) in entries {
            entry.fetched_at = now;
            merged.entries.insert(id, entry);
        }
        merged.entries.retain(|_, entry| {
            now.saturating_sub(entry.fetched_at) <= XCLOUD_CATALOG_BASE_RENDERABLE_TTL_MS
        });
        let snapshot = CachedXcloudCatalogBaseSnapshot {
            updated_at: now,
            entries: merged.entries,
        };

        let store = self.settings_store.open_write()?;
        store
            .store()
            .set(Self::base_store_key(scope), json!(snapshot.clone()));
        store.save()?;

        self.memory_base_cache_guard()
            .insert(Self::scoped_cache_key(scope), snapshot);
        Ok(())
    }

    pub(crate) fn save_xcloud_catalog_overlay(
        &self,
        scope: &XcloudCatalogCacheScope,
        entries: Vec<CachedXcloudCatalogOverlayEntry>,
    ) -> Result<(), String> {
        let snapshot = CachedXcloudCatalogOverlaySnapshot {
            updated_at: Self::now_ms(),
            entries,
        };

        let store = self.settings_store.open_write()?;
        store
            .store()
            .set(Self::overlay_store_key(scope), json!(snapshot.clone()));
        store.save()?;

        self.memory_overlay_cache_guard()
            .insert(Self::scoped_cache_key(scope), snapshot);
        Ok(())
    }

    pub(crate) fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0)
    }

    pub(crate) fn dynamic_cache_prefix() -> &'static str {
        XCLOUD_CATALOG_CACHE_PREFIX
    }

    fn build_loaded_snapshot(
        base_snapshot: Option<&CachedXcloudCatalogBaseSnapshot>,
        overlay_snapshot: Option<&CachedXcloudCatalogOverlaySnapshot>,
        hit_level: &'static str,
    ) -> LoadedXcloudCatalogSnapshot {
        let Some(overlay_snapshot) = overlay_snapshot else {
            return LoadedXcloudCatalogSnapshot {
                titles: Vec::new(),
                updated_at: None,
                cache_state: DataXcloudCatalogCacheState::Miss,
                needs_refresh: true,
                missing_product_ids: Vec::new(),
                hit_level,
            };
        };

        if !Self::is_overlay_renderable(overlay_snapshot) {
            return LoadedXcloudCatalogSnapshot {
                titles: Vec::new(),
                updated_at: None,
                cache_state: DataXcloudCatalogCacheState::Miss,
                needs_refresh: true,
                missing_product_ids: Vec::new(),
                hit_level,
            };
        }

        let base_entries = base_snapshot
            .filter(|snapshot| Self::is_base_renderable(snapshot))
            .map(|snapshot| &snapshot.entries);
        let mut titles = Vec::new();
        let mut missing_product_ids = Vec::new();

        for overlay in &overlay_snapshot.entries {
            let base_entry = base_entries
                .and_then(|entries| entries.get(&overlay.product_id))
                .filter(|entry| {
                    entry.fetched_at == 0
                        || Self::now_ms().saturating_sub(entry.fetched_at)
                            <= XCLOUD_CATALOG_BASE_RENDERABLE_TTL_MS
                });
            if base_entry.is_none() {
                missing_product_ids.push(overlay.product_id.clone());
            }

            let name = base_entry
                .map(|entry| entry.name.clone())
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    let fallback_name = overlay.fallback_name.trim().to_string();
                    if fallback_name.is_empty() {
                        None
                    } else {
                        Some(fallback_name)
                    }
                })
                .unwrap_or_else(|| overlay.product_id.clone());

            titles.push(DataXcloudTitleSummary {
                aliases: vec![overlay.fallback_name.clone()],
                entitlement_known: overlay.entitlement_known,
                recent_rank: overlay.recent_rank,
                new_rank: overlay.new_rank,
                popular_rank: overlay.popular_rank,
                leaving_rank: overlay.leaving_rank,
                id: overlay.product_id.clone(),
                name,
                product_id: overlay.product_id.clone(),
                title_id: overlay.title_id.clone(),
                xbox_title_id: overlay.xbox_title_id,
                publisher_name: base_entry
                    .map(|entry| entry.publisher_name.clone())
                    .unwrap_or_default(),
                description: base_entry
                    .map(|entry| entry.description.clone())
                    .unwrap_or_default(),
                tile_image_url: base_entry
                    .map(|entry| entry.tile_image_url.clone())
                    .unwrap_or_default(),
                poster_image_url: base_entry
                    .map(|entry| entry.poster_image_url.clone())
                    .unwrap_or_default(),
                categories: base_entry
                    .map(|entry| entry.categories.clone())
                    .unwrap_or_default(),
                supported_input_types: overlay.supported_input_types.clone(),
                has_entitlement: overlay.has_entitlement,
                is_recently_played: overlay.is_recently_played,
                is_new: overlay.is_new,
            });
        }

        titles.sort_by(|left, right| left.name.cmp(&right.name));

        let overlay_fresh = Self::is_overlay_fresh(overlay_snapshot);
        let cache_state = if overlay_fresh && missing_product_ids.is_empty() {
            DataXcloudCatalogCacheState::Fresh
        } else {
            DataXcloudCatalogCacheState::Stale
        };

        let updated_at = Some(
            base_snapshot
                .map(|snapshot| snapshot.updated_at)
                .unwrap_or(0)
                .max(overlay_snapshot.updated_at),
        );

        LoadedXcloudCatalogSnapshot {
            titles,
            updated_at,
            cache_state,
            needs_refresh: !overlay_fresh || !missing_product_ids.is_empty(),
            missing_product_ids,
            hit_level,
        }
    }

    fn get_memory_catalog_pair(
        &self,
        scoped_key: &str,
    ) -> Option<(
        Option<CachedXcloudCatalogBaseSnapshot>,
        Option<CachedXcloudCatalogOverlaySnapshot>,
    )> {
        let base_snapshot = self.memory_base_cache_guard().get(scoped_key).cloned();
        let overlay_snapshot = self.memory_overlay_cache_guard().get(scoped_key).cloned();
        if base_snapshot.is_none() && overlay_snapshot.is_none() {
            return None;
        }
        Some((base_snapshot, overlay_snapshot))
    }

    fn cache_catalog_pair(
        &self,
        scoped_key: &str,
        base_snapshot: &Option<CachedXcloudCatalogBaseSnapshot>,
        overlay_snapshot: &Option<CachedXcloudCatalogOverlaySnapshot>,
    ) {
        if let Some(base_snapshot) = base_snapshot {
            self.memory_base_cache_guard()
                .insert(scoped_key.to_string(), base_snapshot.clone());
        }
        if let Some(overlay_snapshot) = overlay_snapshot {
            self.memory_overlay_cache_guard()
                .insert(scoped_key.to_string(), overlay_snapshot.clone());
        }
    }

    fn read_disk_catalog_pair(
        &self,
        scope: &XcloudCatalogCacheScope,
    ) -> Result<
        (
            Option<CachedXcloudCatalogBaseSnapshot>,
            Option<CachedXcloudCatalogOverlaySnapshot>,
        ),
        String,
    > {
        let store = self.settings_store.open_read()?;
        let base_snapshot = store
            .store()
            .get(Self::base_store_key(scope))
            .and_then(|value| serde_json::from_value(value.clone()).ok());
        let overlay_snapshot = store
            .store()
            .get(Self::overlay_store_key(scope))
            .and_then(|value| serde_json::from_value(value.clone()).ok());
        if Self::is_overlay_snapshot_usable(overlay_snapshot.as_ref()) {
            return Ok((base_snapshot, overlay_snapshot));
        }

        Ok(Self::find_compatible_catalog_pair(&store, scope)
            .unwrap_or((base_snapshot, overlay_snapshot)))
    }

    fn find_compatible_catalog_pair(
        store: &crate::settings_store::ResolvedSettingsStore,
        scope: &XcloudCatalogCacheScope,
    ) -> Option<(
        Option<CachedXcloudCatalogBaseSnapshot>,
        Option<CachedXcloudCatalogOverlaySnapshot>,
    )> {
        let expected_stem = Self::scoped_cache_key(scope);
        let entries = store.store().entries();
        let (_stem, overlay_snapshot, base_snapshot) =
            Self::select_compatible_catalog_pair_from_entries(&entries, &expected_stem)?;

        log::info!("[Data][xcloud] using scoped catalog cache fallback");
        Some((base_snapshot, Some(overlay_snapshot)))
    }

    fn select_compatible_catalog_pair_from_entries(
        entries: &[(String, Value)],
        expected_stem: &str,
    ) -> Option<(
        String,
        CachedXcloudCatalogOverlaySnapshot,
        Option<CachedXcloudCatalogBaseSnapshot>,
    )> {
        let mut best: Option<(
            String,
            CachedXcloudCatalogOverlaySnapshot,
            Option<CachedXcloudCatalogBaseSnapshot>,
        )> = None;

        for (key, value) in entries {
            let Some(stem) = key.strip_suffix(".overlay") else {
                continue;
            };
            // 最近游玩与资格 overlay 按完整账户范围匹配；商品基础资料继续复用独立缓存。
            if stem != expected_stem {
                continue;
            }

            let Ok(overlay_snapshot) =
                serde_json::from_value::<CachedXcloudCatalogOverlaySnapshot>(value.clone())
            else {
                continue;
            };
            if !Self::is_overlay_snapshot_usable(Some(&overlay_snapshot)) {
                continue;
            }

            let base_key = format!("{stem}.base");
            let base_snapshot = entries
                .iter()
                .find(|(key, _)| key == &base_key)
                .map(|(_, value)| value)
                .and_then(|value| serde_json::from_value(value.clone()).ok());
            let should_replace = best
                .as_ref()
                .map(|(_, best_overlay, _)| overlay_snapshot.updated_at > best_overlay.updated_at)
                .unwrap_or(true);
            if should_replace {
                best = Some((stem.to_string(), overlay_snapshot, base_snapshot));
            }
        }

        best
    }

    fn find_compatible_base_snapshot(
        store: &crate::settings_store::ResolvedSettingsStore,
        scope: &XcloudCatalogCacheScope,
    ) -> Option<CachedXcloudCatalogBaseSnapshot> {
        let suffix = Self::compatible_scope_suffix(scope);
        let mut best: Option<CachedXcloudCatalogBaseSnapshot> = None;

        for (key, value) in store.store().entries() {
            let Some(stem) = key.strip_suffix(".base") else {
                continue;
            };
            if !stem.starts_with(XCLOUD_CATALOG_CACHE_PREFIX) || !stem.ends_with(&suffix) {
                continue;
            }

            let Ok(snapshot) =
                serde_json::from_value::<CachedXcloudCatalogBaseSnapshot>(value.clone())
            else {
                continue;
            };
            if !Self::is_base_renderable(&snapshot) {
                continue;
            }

            let should_replace = best
                .as_ref()
                .map(|best_snapshot| snapshot.updated_at > best_snapshot.updated_at)
                .unwrap_or(true);
            if should_replace {
                best = Some(snapshot);
            }
        }

        best
    }

    fn is_overlay_snapshot_usable(snapshot: Option<&CachedXcloudCatalogOverlaySnapshot>) -> bool {
        snapshot
            .filter(|snapshot| Self::is_overlay_renderable(snapshot))
            .is_some()
    }

    fn compatible_scope_suffix(scope: &XcloudCatalogCacheScope) -> String {
        format!(
            ".{}.{}.{}",
            Self::sanitize_cache_segment(&scope.region_host),
            Self::sanitize_cache_segment(&scope.language),
            Self::sanitize_cache_segment(&scope.market),
        )
    }

    fn memory_base_cache_guard(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<String, CachedXcloudCatalogBaseSnapshot>> {
        self.memory_base_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn memory_overlay_cache_guard(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<String, CachedXcloudCatalogOverlaySnapshot>> {
        self.memory_overlay_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn is_base_renderable(snapshot: &CachedXcloudCatalogBaseSnapshot) -> bool {
        Self::now_ms().saturating_sub(snapshot.updated_at) <= XCLOUD_CATALOG_BASE_RENDERABLE_TTL_MS
    }

    fn is_overlay_renderable(snapshot: &CachedXcloudCatalogOverlaySnapshot) -> bool {
        Self::now_ms().saturating_sub(snapshot.updated_at)
            <= XCLOUD_CATALOG_OVERLAY_RENDERABLE_TTL_MS
    }

    fn is_overlay_fresh(snapshot: &CachedXcloudCatalogOverlaySnapshot) -> bool {
        Self::now_ms().saturating_sub(snapshot.updated_at) <= XCLOUD_CATALOG_OVERLAY_FRESH_TTL_MS
    }

    pub(crate) fn scoped_cache_key(scope: &XcloudCatalogCacheScope) -> String {
        format!(
            "{}.{}.{}.{}.{}",
            XCLOUD_CATALOG_CACHE_PREFIX,
            Self::sanitize_cache_segment(&scope.account_id),
            Self::sanitize_cache_segment(&scope.region_host),
            Self::sanitize_cache_segment(&scope.language),
            Self::sanitize_cache_segment(&scope.market),
        )
    }

    fn base_store_key(scope: &XcloudCatalogCacheScope) -> String {
        format!("{}.base", Self::scoped_cache_key(scope))
    }

    fn overlay_store_key(scope: &XcloudCatalogCacheScope) -> String {
        format!("{}.overlay", Self::scoped_cache_key(scope))
    }

    fn sanitize_cache_segment(value: &str) -> String {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return "unknown".to_string();
        }

        trimmed
            .chars()
            .map(|ch| match ch {
                'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => ch,
                '.' | ':' | '/' | '\\' => '-',
                _ => '_',
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_empty_catalog_is_a_fresh_authoritative_snapshot() {
        let empty = CachedXcloudCatalogOverlaySnapshot {
            updated_at: DataCacheRepository::now_ms(),
            entries: vec![],
        };
        assert!(DataCacheRepository::is_overlay_snapshot_usable(Some(
            &empty
        )));
        let loaded = DataCacheRepository::build_loaded_snapshot(None, Some(&empty), "memory");
        assert_eq!(loaded.cache_state, DataXcloudCatalogCacheState::Fresh);
        assert!(!loaded.needs_refresh);
        assert!(loaded.titles.is_empty());
    }
    #[test]
    fn refreshing_other_metadata_keeps_old_products_expired() {
        let now = DataCacheRepository::now_ms();
        let base = CachedXcloudCatalogBaseSnapshot {
            updated_at: now,
            entries: HashMap::from([
                (
                    "A".into(),
                    CachedXcloudCatalogBaseEntry {
                        product_id: "A".into(),
                        name: "Old".into(),
                        fetched_at: now - XCLOUD_CATALOG_BASE_RENDERABLE_TTL_MS - 1,
                        ..Default::default()
                    },
                ),
                (
                    "B".into(),
                    CachedXcloudCatalogBaseEntry {
                        product_id: "B".into(),
                        name: "New".into(),
                        fetched_at: now,
                        ..Default::default()
                    },
                ),
            ]),
        };
        let overlay = CachedXcloudCatalogOverlaySnapshot {
            updated_at: now,
            entries: vec![
                CachedXcloudCatalogOverlayEntry {
                    product_id: "A".into(),
                    fallback_name: "Service A".into(),
                    ..Default::default()
                },
                CachedXcloudCatalogOverlayEntry {
                    product_id: "B".into(),
                    ..Default::default()
                },
            ],
        };
        let loaded =
            DataCacheRepository::build_loaded_snapshot(Some(&base), Some(&overlay), "memory");
        assert_eq!(loaded.missing_product_ids, vec!["A"]);
        assert_eq!(
            loaded
                .titles
                .iter()
                .find(|t| t.product_id == "A")
                .unwrap()
                .name,
            "Service A"
        );
    }

    #[test]
    fn stale_overlay_stays_renderable_and_marks_missing_products() {
        let base_snapshot = CachedXcloudCatalogBaseSnapshot {
            updated_at: DataCacheRepository::now_ms(),
            entries: HashMap::from([(
                "A".to_string(),
                CachedXcloudCatalogBaseEntry {
                    fetched_at: 0,

                    product_id: "A".to_string(),
                    name: "Alpha".to_string(),
                    publisher_name: String::new(),
                    description: String::new(),
                    tile_image_url: String::new(),
                    poster_image_url: String::new(),
                    categories: Vec::new(),
                },
            )]),
        };
        let overlay_snapshot = CachedXcloudCatalogOverlaySnapshot {
            updated_at: DataCacheRepository::now_ms() - (XCLOUD_CATALOG_OVERLAY_FRESH_TTL_MS + 1),
            entries: vec![
                CachedXcloudCatalogOverlayEntry {
                    entitlement_known: false,
                    recent_rank: None,
                    new_rank: None,
                    popular_rank: None,
                    leaving_rank: None,

                    product_id: "A".to_string(),
                    title_id: "title-a".to_string(),
                    xbox_title_id: Some(1),
                    fallback_name: "Alpha".to_string(),
                    supported_input_types: vec!["controller".to_string()],
                    has_entitlement: true,
                    is_recently_played: false,
                    is_new: false,
                },
                CachedXcloudCatalogOverlayEntry {
                    entitlement_known: false,
                    recent_rank: None,
                    new_rank: None,
                    popular_rank: None,
                    leaving_rank: None,

                    product_id: "B".to_string(),
                    title_id: "title-b".to_string(),
                    xbox_title_id: Some(2),
                    fallback_name: "Bravo".to_string(),
                    supported_input_types: vec!["touch".to_string()],
                    has_entitlement: true,
                    is_recently_played: true,
                    is_new: false,
                },
            ],
        };

        let loaded = DataCacheRepository::build_loaded_snapshot(
            Some(&base_snapshot),
            Some(&overlay_snapshot),
            "disk",
        );

        assert_eq!(loaded.cache_state, DataXcloudCatalogCacheState::Stale);
        assert!(loaded.needs_refresh);
        assert_eq!(loaded.missing_product_ids, vec!["B".to_string()]);
        assert_eq!(loaded.titles.len(), 2);
    }

    #[test]
    fn catalog_overlay_fallback_preserves_account_and_region_scope() {
        let old_stem =
            "data.xcloudCatalog.v2.old-user.wus-core-gssv-play-prod-xboxlive-com.en-US.US";
        let new_stem =
            "data.xcloudCatalog.v2.new-user.wus-core-gssv-play-prod-xboxlive-com.en-US.US";
        let other_region_stem =
            "data.xcloudCatalog.v2.other-user.ejp-core-gssv-play-prod-xboxlive-com.en-US.US";
        let now = DataCacheRepository::now_ms();

        let entries = vec![
            (
                format!("{new_stem}.base"),
                serde_json::json!(CachedXcloudCatalogBaseSnapshot {
                    updated_at: now,
                    entries: HashMap::from([(
                        "NEW".to_string(),
                        CachedXcloudCatalogBaseEntry {
                            fetched_at: 0,

                            product_id: "NEW".to_string(),
                            name: "Newest".to_string(),
                            publisher_name: String::new(),
                            description: String::new(),
                            tile_image_url: String::new(),
                            poster_image_url: String::new(),
                            categories: Vec::new(),
                        }
                    )]),
                }),
            ),
            (
                format!("{old_stem}.overlay"),
                serde_json::json!(CachedXcloudCatalogOverlaySnapshot {
                    updated_at: now - 10,
                    entries: vec![CachedXcloudCatalogOverlayEntry {
                        entitlement_known: false,
                        recent_rank: None,
                        new_rank: None,
                        popular_rank: None,
                        leaving_rank: None,

                        product_id: "OLD".to_string(),
                        title_id: "old-title".to_string(),
                        xbox_title_id: None,
                        fallback_name: "Old".to_string(),
                        supported_input_types: Vec::new(),
                        has_entitlement: true,
                        is_recently_played: false,
                        is_new: false,
                    }],
                }),
            ),
            (
                format!("{new_stem}.overlay"),
                serde_json::json!(CachedXcloudCatalogOverlaySnapshot {
                    updated_at: now,
                    entries: vec![CachedXcloudCatalogOverlayEntry {
                        entitlement_known: false,
                        recent_rank: None,
                        new_rank: None,
                        popular_rank: None,
                        leaving_rank: None,

                        product_id: "NEW".to_string(),
                        title_id: "new-title".to_string(),
                        xbox_title_id: None,
                        fallback_name: "Newest".to_string(),
                        supported_input_types: Vec::new(),
                        has_entitlement: true,
                        is_recently_played: false,
                        is_new: false,
                    }],
                }),
            ),
            (
                format!("{other_region_stem}.overlay"),
                serde_json::json!(CachedXcloudCatalogOverlaySnapshot {
                    updated_at: now + 1,
                    entries: vec![CachedXcloudCatalogOverlayEntry {
                        entitlement_known: false,
                        recent_rank: None,
                        new_rank: None,
                        popular_rank: None,
                        leaving_rank: None,

                        product_id: "OTHER".to_string(),
                        title_id: "other-title".to_string(),
                        xbox_title_id: None,
                        fallback_name: "Other".to_string(),
                        supported_input_types: Vec::new(),
                        has_entitlement: true,
                        is_recently_played: false,
                        is_new: false,
                    }],
                }),
            ),
        ];

        let (stem, overlay, base) =
            DataCacheRepository::select_compatible_catalog_pair_from_entries(&entries, new_stem)
                .expect("compatible overlay should be selected");

        assert_eq!(stem, new_stem);
        assert_eq!(overlay.entries[0].product_id, "NEW");
        assert_eq!(
            base.expect("matching base should be included")
                .entries
                .get("NEW")
                .map(|entry| entry.name.as_str()),
            Some("Newest")
        );
        assert!(
            DataCacheRepository::select_compatible_catalog_pair_from_entries(
                &entries,
                "data.xcloudCatalog.v2.missing-user.wus-core-gssv-play-prod-xboxlive-com.en-US.US",
            )
            .is_none()
        );
    }

    #[test]
    fn sanitize_scope_replaces_unstable_characters() {
        let scope = XcloudCatalogCacheScope {
            account_id: "user:1".to_string(),
            region_host: "eastus.xbox.com".to_string(),
            language: "zh-TW".to_string(),
            market: "US".to_string(),
        };

        let scoped_key = DataCacheRepository::scoped_cache_key(&scope);
        assert!(scoped_key.contains("user-1"));
        assert!(scoped_key.contains("eastus-xbox-com"));
    }

    #[test]
    fn profile_cache_requires_matching_account_and_ignores_legacy_unscoped_data() {
        let cached = json!({"accountScope":"account-a", "settings":{"Gamertag":"Alpha"}});
        assert_eq!(
            scoped_profile_settings(&cached, Some("account-a"))
                .get("Gamertag")
                .map(String::as_str),
            Some("Alpha")
        );
        assert!(scoped_profile_settings(&cached, Some("account-b")).is_empty());
        assert!(scoped_profile_settings(&cached, None).is_empty());
        assert!(
            scoped_profile_settings(&json!({"Gamertag":"Legacy"}), Some("account-a")).is_empty()
        );
    }
}
