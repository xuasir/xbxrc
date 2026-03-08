use serde_json::{Map, Value};
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

pub struct ConfigRepository {
    app_handle: AppHandle,
}

impl ConfigRepository {
    pub fn new(app_handle: AppHandle) -> Self {
        Self { app_handle }
    }

    pub fn get_all_settings(&self) -> Result<Map<String, Value>, String> {
        let store = self
            .app_handle
            .store("settings.json")
            .map_err(|error| error.to_string())?;

        let mut settings = Map::new();
        for (key, value) in store.entries() {
            settings.insert(key.to_string(), value.clone());
        }

        Ok(settings)
    }

    pub fn set_by_patch(&self, patch: &Map<String, Value>) -> Result<(), String> {
        let store = self
            .app_handle
            .store("settings.json")
            .map_err(|error| error.to_string())?;

        for (key, value) in patch {
            store.set(key, value.clone());
        }

        store.save().map_err(|error| error.to_string())
    }
}
