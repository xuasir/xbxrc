use serde_json::{Map, Value};
use std::sync::Arc;
use tauri::AppHandle;
use tauri_plugin_store::{resolve_store_path, Error as StoreError, Store, StoreExt};

const PRIMARY_SETTINGS_STORE: &str = "settings.json";
const FALLBACK_SETTINGS_STORE: &str = "settings.v2.json";

pub struct ConfigRepository {
    app_handle: AppHandle,
}

impl ConfigRepository {
    pub fn new(app_handle: AppHandle) -> Self {
        Self { app_handle }
    }

    fn fallback_store_exists(&self) -> bool {
        resolve_store_path(&self.app_handle, FALLBACK_SETTINGS_STORE)
            .map(|path| path.exists())
            .unwrap_or(false)
    }

    fn is_permission_error(error: &StoreError) -> bool {
        matches!(error, StoreError::Io(io_error) if io_error.kind() == std::io::ErrorKind::PermissionDenied)
            || error.to_string().contains("Operation not permitted")
    }

    fn open_read_store(&self) -> Result<Arc<Store<tauri::Wry>>, String> {
        let store_path = if self.fallback_store_exists() {
            FALLBACK_SETTINGS_STORE
        } else {
            PRIMARY_SETTINGS_STORE
        };
        self.app_handle
            .store(store_path)
            .map_err(|error| format!("Failed to open store '{}': {}", store_path, error))
    }

    fn open_write_store(&self) -> Result<Arc<Store<tauri::Wry>>, String> {
        // 如果回退存储已存在，优先使用回退文件，避免反复触发主文件权限错误。
        if self.fallback_store_exists() {
            return self
                .app_handle
                .store(FALLBACK_SETTINGS_STORE)
                .map_err(|error| {
                    format!(
                        "Failed to open fallback store '{}': {}",
                        FALLBACK_SETTINGS_STORE, error
                    )
                });
        }

        let primary_store = self
            .app_handle
            .store(PRIMARY_SETTINGS_STORE)
            .map_err(|error| {
                format!(
                    "Failed to open primary store '{}': {}",
                    PRIMARY_SETTINGS_STORE, error
                )
            })?;

        // 预保存用于提前探测文件系统可写性，避免真正写 patch 时才抛错。
        if let Err(error) = primary_store.save() {
            if Self::is_permission_error(&error) {
                log::warn!(
                    "Primary store '{}' is not writable ({}), fallback to '{}'",
                    PRIMARY_SETTINGS_STORE,
                    error,
                    FALLBACK_SETTINGS_STORE
                );

                let fallback_store =
                    self.app_handle
                        .store(FALLBACK_SETTINGS_STORE)
                        .map_err(|open_error| {
                            format!(
                                "Failed to open fallback store '{}': {} (primary save error: {})",
                                FALLBACK_SETTINGS_STORE, open_error, error
                            )
                        })?;

                // 尝试迁移当前内存中的主存储数据，降低切换后的配置丢失风险。
                for (key, value) in primary_store.entries() {
                    fallback_store.set(key, value);
                }

                fallback_store.save().map_err(|save_error| {
                    format!(
                        "Failed to save fallback store '{}': {} (primary save error: {})",
                        FALLBACK_SETTINGS_STORE, save_error, error
                    )
                })?;

                return Ok(fallback_store);
            }

            return Err(format!(
                "Failed to save primary store '{}': {}",
                PRIMARY_SETTINGS_STORE, error
            ));
        }

        Ok(primary_store)
    }

    pub fn get_all_settings(&self) -> Result<Map<String, Value>, String> {
        let store = self.open_read_store()?;

        let mut settings = Map::new();
        for (key, value) in store.entries() {
            settings.insert(key.to_string(), value.clone());
        }

        Ok(settings)
    }

    pub fn set_by_patch(&self, patch: &Map<String, Value>) -> Result<(), String> {
        let store = self.open_write_store()?;

        for (key, value) in patch {
            store.set(key, value.clone());
        }

        store.save().map_err(|error| {
            format!(
                "Failed to save store '{}': {}",
                if self.fallback_store_exists() {
                    FALLBACK_SETTINGS_STORE
                } else {
                    PRIMARY_SETTINGS_STORE
                },
                error
            )
        })
    }
}
