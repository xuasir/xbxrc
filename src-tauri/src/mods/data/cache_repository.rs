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

impl DataCacheRepository {
    pub fn new(app_handle: AppHandle) -> Self {
        Self {
            settings_store: SettingsStoreResolver::new(app_handle),
            memory_base_cache: Mutex::new(HashMap::new()),
            memory_overlay_cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn save_cached_profile(&self, profile: &DataUserProfile) -> Result<(), String> {
        let mut profile_patch = Map::new();
        for (key, value) in &profile.settings {
            profile_patch.insert(key.clone(), Value::String(value.clone()));
        }

        let store = self.settings_store.open_write()?;
        store
            .store()
            .set(PROFILE_CACHE_KEY, Value::Object(profile_patch));
        store.save()
    }

    pub fn clear_cached_profile(&self) -> Result<(), String> {
        let store = self.settings_store.open_write()?;
        store.store().delete(PROFILE_CACHE_KEY);
        store.save()
    }

    pub fn get_cached_profile(&self, app_level: u32) -> Result<DataUserProfile, String> {
        let store = self.settings_store.open_read()?;

        let settings_value = store.store().get(PROFILE_CACHE_KEY).unwrap_or(Value::Null);
        let mut settings = HashMap::new();

        if let Some(object) = settings_value.as_object() {
            for (key, value) in object {
                if let Some(value_str) = value.as_str() {
                    settings.insert(key.to_string(), value_str.to_string());
                }
            }
        }

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
        if let Some(snapshot) = &snapshot {
            self.memory_base_cache_guard()
                .insert(scoped_key, snapshot.clone());
        }
        Ok(snapshot)
    }

    pub(crate) fn save_xcloud_catalog_base(
        &self,
        scope: &XcloudCatalogCacheScope,
        entries: HashMap<String, CachedXcloudCatalogBaseEntry>,
    ) -> Result<(), String> {
        let snapshot = CachedXcloudCatalogBaseSnapshot {
            updated_at: Self::now_ms(),
            entries,
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
        if entries.is_empty() {
            return Ok(());
        }

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

        if !Self::is_overlay_renderable(overlay_snapshot) || overlay_snapshot.entries.is_empty() {
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
            let base_entry = base_entries.and_then(|entries| entries.get(&overlay.product_id));
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
        Ok((base_snapshot, overlay_snapshot))
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
    fn stale_overlay_stays_renderable_and_marks_missing_products() {
        let base_snapshot = CachedXcloudCatalogBaseSnapshot {
            updated_at: DataCacheRepository::now_ms(),
            entries: HashMap::from([(
                "A".to_string(),
                CachedXcloudCatalogBaseEntry {
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
}
