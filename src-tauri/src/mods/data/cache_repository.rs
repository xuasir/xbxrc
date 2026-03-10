use crate::mods::data::types::{DataUserProfile, DataXcloudTitleSummary};
use crate::settings_store::SettingsStoreResolver;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::AppHandle;

const PROFILE_CACHE_KEY: &str = "data.profileCache";
const XCLOUD_TITLES_CACHE_KEY: &str = "data.xcloudTitlesCache";
const XCLOUD_TITLES_CACHE_STALE_MAX_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone)]
pub(crate) struct CachedXcloudTitles {
    pub updated_at: u64,
    pub titles: Vec<DataXcloudTitleSummary>,
}

pub struct DataCacheRepository {
    settings_store: SettingsStoreResolver,
    xcloud_titles_cache: Mutex<Option<CachedXcloudTitles>>,
}

impl DataCacheRepository {
    pub fn new(app_handle: AppHandle) -> Self {
        Self {
            settings_store: SettingsStoreResolver::new(app_handle),
            xcloud_titles_cache: Mutex::new(None),
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

    pub(crate) fn get_cached_xcloud_titles(&self) -> Result<Option<CachedXcloudTitles>, String> {
        if let Some(cache) = self.cached_xcloud_titles() {
            if Self::is_xcloud_cache_usable(&cache) {
                return Ok(Some(cache));
            }
        }

        let store = self.settings_store.open_read()?;
        let Some(raw) = store.store().get(XCLOUD_TITLES_CACHE_KEY) else {
            return Ok(None);
        };

        let Some(object) = raw.as_object() else {
            return Ok(None);
        };

        let updated_at = object
            .get("updatedAt")
            .and_then(|value| value.as_u64())
            .unwrap_or(0);
        let Some(raw_titles) = object.get("titles").and_then(|value| value.as_array()) else {
            return Ok(None);
        };

        let titles = raw_titles
            .iter()
            .filter_map(|item| serde_json::from_value::<DataXcloudTitleSummary>(item.clone()).ok())
            .collect::<Vec<_>>();

        if titles.is_empty() {
            return Ok(None);
        }

        let payload = CachedXcloudTitles { updated_at, titles };
        if !Self::is_xcloud_cache_usable(&payload) {
            return Ok(None);
        }

        *self.xcloud_titles_cache_guard() = Some(payload.clone());
        Ok(Some(payload))
    }

    pub(crate) fn set_cached_xcloud_titles(
        &self,
        titles: Vec<DataXcloudTitleSummary>,
    ) -> Result<(), String> {
        if titles.is_empty() {
            return Ok(());
        }

        let payload = CachedXcloudTitles {
            updated_at: Self::now_ms(),
            titles,
        };

        let serialized_payload = json!({
            "updatedAt": payload.updated_at,
            "titles": payload.titles
        });

        let store = self.settings_store.open_write()?;
        store
            .store()
            .set(XCLOUD_TITLES_CACHE_KEY, serialized_payload);
        store.save()?;

        *self.xcloud_titles_cache_guard() = Some(payload);
        Ok(())
    }

    fn cached_xcloud_titles(&self) -> Option<CachedXcloudTitles> {
        self.xcloud_titles_cache_guard().clone()
    }

    fn xcloud_titles_cache_guard(&self) -> std::sync::MutexGuard<'_, Option<CachedXcloudTitles>> {
        self.xcloud_titles_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn is_xcloud_cache_usable(payload: &CachedXcloudTitles) -> bool {
        Self::now_ms().saturating_sub(payload.updated_at) <= XCLOUD_TITLES_CACHE_STALE_MAX_MS
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0)
    }
}
