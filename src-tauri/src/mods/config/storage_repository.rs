use crate::settings_store::SettingsStoreResolver;
use serde_json::{Map, Value};
use tauri::AppHandle;

pub struct ConfigStorageRepository {
    settings_store: SettingsStoreResolver,
}

impl ConfigStorageRepository {
    pub fn new(app_handle: AppHandle) -> Self {
        Self {
            settings_store: SettingsStoreResolver::new(app_handle),
        }
    }

    pub fn get_all_settings(&self) -> Result<Map<String, Value>, String> {
        let store = self.settings_store.open_read()?;

        let mut settings = Map::new();
        for (key, value) in store.store().entries() {
            settings.insert(key.to_string(), value.clone());
        }

        Ok(settings)
    }

    pub fn set_by_patch(&self, patch: &Map<String, Value>) -> Result<(), String> {
        let store = self.settings_store.open_write()?;

        for (key, value) in patch {
            store.store().set(key, value.clone());
        }

        store.save()
    }
}
