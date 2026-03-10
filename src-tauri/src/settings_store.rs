use std::sync::Arc;

use tauri::AppHandle;
use tauri_plugin_store::{Store, StoreExt};

const CANONICAL_SETTINGS_STORE: &str = "settings.xbx.json";

pub struct ResolvedSettingsStore {
    store: Arc<Store<tauri::Wry>>,
    store_name: &'static str,
}

impl ResolvedSettingsStore {
    pub fn store(&self) -> &Arc<Store<tauri::Wry>> {
        &self.store
    }

    pub fn save(&self) -> Result<(), String> {
        self.store
            .save()
            .map_err(|error| format!("Failed to save store '{}': {}", self.store_name, error))
    }
}

pub struct SettingsStoreResolver {
    app_handle: AppHandle,
}

impl SettingsStoreResolver {
    pub fn new(app_handle: AppHandle) -> Self {
        Self { app_handle }
    }

    pub fn open_read(&self) -> Result<ResolvedSettingsStore, String> {
        self.open_canonical_store()
    }

    pub fn open_write(&self) -> Result<ResolvedSettingsStore, String> {
        self.open_canonical_store()
    }

    fn open_canonical_store(&self) -> Result<ResolvedSettingsStore, String> {
        let store = self
            .app_handle
            .store(CANONICAL_SETTINGS_STORE)
            .map_err(|error| {
                format!(
                    "Failed to open store '{}': {}",
                    CANONICAL_SETTINGS_STORE, error
                )
            })?;

        Ok(ResolvedSettingsStore {
            store,
            store_name: CANONICAL_SETTINGS_STORE,
        })
    }
}
